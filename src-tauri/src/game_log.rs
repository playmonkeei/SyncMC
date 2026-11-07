//! Convert Minecraft's XML console events into bounded, readable log entries.
use quick_xml::{events::Event, Reader, XmlVersion};
use regex::Regex;
use serde::Serialize;
use std::sync::OnceLock;

const MAX_EVENT_BYTES: usize = 256 * 1024;
pub const MAX_LINE_BYTES: usize = 64 * 1024;
const EVENT_START: &str = "<log4j:Event";
const EVENT_END: &str = "</log4j:Event>";

#[derive(Clone, Debug, Serialize)]
pub struct GameLog {
    pub timestamp: u64,
    pub stream: String,
    pub line: String,
    pub level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

impl GameLog {
    pub fn plain(stream: &str, line: &str, timestamp: u64) -> Self {
        static TEXT_LOG: OnceLock<Regex> = OnceLock::new();
        let pattern = TEXT_LOG.get_or_init(|| {
            Regex::new(r"^\[(?:\d{2}:\d{2}:\d{2}(?:\.\d+)?|\d{2}[A-Za-z]{3}\d{4} \d{2}:\d{2}:\d{2}(?:\.\d+)?)\] \[([^\]]+)/(TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL)\](?: \[([^\]]+)\])?: ?(.*)$").unwrap()
        });
        let mut entry = Self {
            timestamp,
            stream: stream.into(),
            line: line.into(),
            level: if stream == "launcher" {
                "INFO"
            } else {
                "UNKNOWN"
            }
            .into(),
            logger: (stream == "launcher").then(|| "SyncMC".into()),
            thread: None,
        };
        if let Some(captures) = pattern.captures(line) {
            entry.thread = Some(captures[1].into());
            entry.level = level(&captures[2]);
            entry.logger = captures.get(3).map(|value| value.as_str().into());
            entry.line = captures[4].into();
        }
        entry
    }

    /// Redact after XML decoding and message assembly, including metadata.
    pub fn redact(&mut self, secret: &str) {
        if secret.is_empty() {
            return;
        }
        self.line = self.line.replace(secret, "[redacted]");
        self.logger = self
            .logger
            .take()
            .map(|value| value.replace(secret, "[redacted]"));
        self.thread = self
            .thread
            .take()
            .map(|value| value.replace(secret, "[redacted]"));
    }

    pub fn formatted(&self) -> String {
        let timestamp = i64::try_from(self.timestamp)
            .ok()
            .and_then(|value| time::OffsetDateTime::from_unix_timestamp(value).ok())
            .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
        let source = self.logger.as_deref().unwrap_or(&self.stream);
        let thread = self
            .thread
            .as_deref()
            .map(|value| format!(" [{value}]"))
            .unwrap_or_default();
        let prefix = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC [{}] [{}]{} ",
            timestamp.year(),
            u8::from(timestamp.month()),
            timestamp.day(),
            timestamp.hour(),
            timestamp.minute(),
            timestamp.second(),
            self.level,
            source,
            thread
        );
        // Indent continuation lines so stack traces and mod trees remain one entry.
        format!("{prefix}{}", self.line.replace('\n', "\n    "))
    }
}

fn level(value: &str) -> String {
    match value {
        "WARNING" => "WARN",
        "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR" | "FATAL" => value,
        _ => "UNKNOWN",
    }
    .into()
}

#[derive(Default)]
struct XmlEvent {
    raw: String,
    in_cdata: bool,
    oversized: bool,
}

pub struct LogParser {
    stream: String,
    xml: Option<XmlEvent>,
}

impl LogParser {
    pub fn new(stream: &str) -> Self {
        Self {
            stream: stream.into(),
            xml: None,
        }
    }

    pub fn line(&mut self, line: &str, timestamp: u64) -> Vec<GameLog> {
        let mut entries = Vec::new();
        self.append(line, timestamp, &mut entries);
        entries
    }

    fn append(&mut self, mut line: &str, timestamp: u64, entries: &mut Vec<GameLog>) {
        loop {
            let starts_event = line
                .trim_start()
                .strip_prefix(EVENT_START)
                .is_some_and(|rest| rest.starts_with(char::is_whitespace) || rest.starts_with('>'));
            if starts_event
                && self
                    .xml
                    .as_ref()
                    .is_some_and(|event| !event.in_cdata || event.oversized)
            {
                // A new event also recovers from a malformed or interrupted predecessor.
                entries.extend(self.finish(timestamp));
            }
            if self.xml.is_none() {
                if !starts_event {
                    entries.push(GameLog::plain(&self.stream, line, timestamp));
                    return;
                }
                self.xml = Some(XmlEvent::default());
            }
            let event = self.xml.as_mut().unwrap();
            let end = event_end(line, &mut event.in_cdata);
            let part = end.map_or(line, |end| &line[..end]);
            if !event.oversized {
                if event.raw.len() + part.len() + 1 > MAX_EVENT_BYTES {
                    event.raw.clear();
                    event.oversized = true;
                    entries.push(GameLog::plain(
                        &self.stream,
                        "[Zu langer Protokolleintrag ausgelassen]",
                        timestamp,
                    ));
                } else {
                    if !event.raw.is_empty() {
                        event.raw.push('\n');
                    }
                    event.raw.push_str(part);
                }
            }
            let Some(end) = end else { return };
            let event = self.xml.take().unwrap();
            if !event.oversized {
                entries.push(
                    parse_xml(&event.raw, &self.stream, timestamp)
                        .unwrap_or_else(|| GameLog::plain(&self.stream, &event.raw, timestamp)),
                );
            }
            line = &line[end..];
            if line.trim().is_empty() {
                return;
            }
        }
    }

    pub fn finish(&mut self, timestamp: u64) -> Vec<GameLog> {
        self.xml
            .take()
            .filter(|event| !event.oversized)
            .map(|event| vec![GameLog::plain(&self.stream, &event.raw, timestamp)])
            .unwrap_or_default()
    }

    pub fn long_line(&mut self, prefix: &str, timestamp: u64) -> Vec<GameLog> {
        // Discard the whole physical line, never a prefix containing part of a token.
        // If it belongs to XML, continue skipping until its event boundary.
        if self.xml.is_none() && prefix.trim_start().starts_with(EVENT_START) {
            self.xml = Some(XmlEvent::default());
        }
        if let Some(event) = self.xml.as_mut() {
            event.raw.clear();
            event.oversized = true;
            event_end(prefix, &mut event.in_cdata);
        }
        vec![GameLog::plain(
            &self.stream,
            "[Zu lange Protokollzeile ausgelassen]",
            timestamp,
        )]
    }

    pub fn discarded_line_tail(&mut self, tail: &str) {
        if tail.contains(EVENT_END) {
            self.xml = None;
        }
    }
}

/// Find event boundaries outside CDATA (the message may itself contain XML).
fn event_end(text: &str, in_cdata: &mut bool) -> Option<usize> {
    let mut position = 0;
    while position < text.len() {
        if *in_cdata {
            let end = text[position..].find("]]>")?;
            position += end + 3;
            *in_cdata = false;
        } else {
            let cdata = text[position..].find("<![CDATA[");
            let end = text[position..].find(EVENT_END);
            match (cdata, end) {
                (Some(cdata), Some(end)) if end < cdata => {
                    return Some(position + end + EVENT_END.len())
                }
                (Some(cdata), _) => {
                    position += cdata + 9;
                    *in_cdata = true;
                }
                (None, Some(end)) => return Some(position + end + EVENT_END.len()),
                (None, None) => return None,
            }
        }
    }
    None
}

fn parse_xml(raw: &str, stream: &str, timestamp: u64) -> Option<GameLog> {
    let mut reader = Reader::from_str(raw);
    let mut entry = GameLog::plain(stream, "", timestamp);
    let mut depth = 0;
    let mut content_depth = None;
    let mut has_message = false;
    let mut closed = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) => {
                if depth == 0 {
                    if tag.name().as_ref() != "log4j:Event" || closed {
                        return None;
                    }
                    for attribute in tag.attributes() {
                        let attribute = attribute.ok()?;
                        let value = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .ok()?
                            .into_owned();
                        match attribute.key.as_ref() {
                            "timestamp" => {
                                // Broken mod metadata must not produce an invalid JS Date in the UI.
                                if let Some(seconds) = value
                                    .parse::<u64>()
                                    .ok()
                                    .map(|value| value / 1000)
                                    .filter(|seconds| {
                                        i64::try_from(*seconds)
                                            .ok()
                                            .and_then(|seconds| {
                                                time::OffsetDateTime::from_unix_timestamp(seconds)
                                                    .ok()
                                            })
                                            .is_some()
                                    })
                                {
                                    entry.timestamp = seconds;
                                }
                            }
                            "level" => entry.level = level(&value),
                            "logger" => entry.logger = Some(value),
                            "thread" => entry.thread = Some(value),
                            _ => {}
                        }
                    }
                } else if depth == 1 {
                    match tag.name().as_ref() {
                        "log4j:Message" | "log4j:Throwable" => {
                            if !entry.line.is_empty() {
                                entry.line.push('\n');
                            }
                            content_depth = Some(depth + 1);
                            has_message = true;
                        }
                        _ => return None, // Preserve unfamiliar event data in the raw fallback.
                    }
                } else {
                    return None;
                }
                depth += 1;
            }
            Event::End(_) => {
                if depth == 0 {
                    return None;
                }
                if content_depth == Some(depth) {
                    content_depth = None;
                }
                depth -= 1;
                if depth == 0 {
                    closed = true;
                }
            }
            Event::Text(text) => {
                if content_depth.is_some() {
                    entry.line.push_str(&text.xml10_content());
                } else if !text.trim().is_empty() {
                    return None;
                }
            }
            Event::CData(text) if content_depth.is_some() => {
                entry.line.push_str(&text.xml10_content())
            }
            Event::GeneralRef(reference) if content_depth.is_some() => {
                if let Some(character) = reference.resolve_char_ref().ok()? {
                    entry.line.push(character);
                } else {
                    entry.line.push_str(match reference.as_ref() {
                        "lt" => "<",
                        "gt" => ">",
                        "amp" => "&",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => return None,
                    });
                }
            }
            Event::Eof => return (closed && depth == 0 && has_message).then_some(entry),
            Event::Comment(_) => {}
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: &str = "  <log4j:Event logger=\"FabricLoader\" timestamp=\"1794043253428\" level=\"INFO\" thread=\"main\">";

    #[test]
    fn multiline_xml_is_one_readable_entry_with_original_metadata() {
        let mut parser = LogParser::new("stdout");
        assert!(parser.line(OPEN, 10).is_empty());
        assert!(parser
            .line("    <log4j:Message><![CDATA[Loading 58 mods:", 10)
            .is_empty());
        assert!(parser.line("\t- fabric-api 0.116.17+1.21.1", 10).is_empty());
        assert!(parser
            .line("\t   \\-- extra]]></log4j:Message>", 10)
            .is_empty());
        let entries = parser.line("  </log4j:Event>", 10);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.timestamp, 1794043253);
        assert_eq!(entry.logger.as_deref(), Some("FabricLoader"));
        assert_eq!(entry.thread.as_deref(), Some("main"));
        assert_eq!(entry.level, "INFO");
        assert_eq!(
            entry.line,
            "Loading 58 mods:\n\t- fabric-api 0.116.17+1.21.1\n\t   \\-- extra"
        );
        assert!(entry.formatted().starts_with(
            "2026-11-07 09:20:53 UTC [INFO] [FabricLoader] [main] Loading 58 mods:\n"
        ));
    }

    #[test]
    fn entities_and_cdata_decode_once_then_redact_including_metadata_and_throwable() {
        let raw = r#"<log4j:Event logger="sec&#114;et" level="ERROR" thread="secret"><log4j:Message>sec&#114;et &amp; <![CDATA[<literal &amp;> </log4j:Event>]]></log4j:Message><log4j:Throwable><![CDATA[java.lang.Exception: secret
\tat Example.run(Example.java:1)]]></log4j:Throwable></log4j:Event>"#;
        let mut entries = LogParser::new("stderr").line(raw, 20);
        assert_eq!(entries.len(), 1);
        entries[0].redact("secret");
        let entry = &entries[0];
        assert_eq!(entry.level, "ERROR");
        assert_eq!(entry.logger.as_deref(), Some("[redacted]"));
        assert_eq!(entry.thread.as_deref(), Some("[redacted]"));
        assert_eq!(entry.line, "[redacted] & <literal &amp;> </log4j:Event>\njava.lang.Exception: [redacted]\n\\tat Example.run(Example.java:1)");
        assert!(!entry.formatted().contains("secret"));
    }

    #[test]
    fn plain_text_output_and_stacktrace_indentation_are_preserved() {
        let mut parser = LogParser::new("stderr");
        let entry = parser
            .line("[10:05:23] [Render thread/WARN]: Missing sound", 15)
            .remove(0);
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.thread.as_deref(), Some("Render thread"));
        assert_eq!(entry.line, "Missing sound");
        let entry = parser
            .line("\tat Example.run(Example.java:12)  ", 15)
            .remove(0);
        assert_eq!(entry.level, "UNKNOWN");
        assert_eq!(entry.line, "\tat Example.run(Example.java:12)  ");
        assert_eq!(
            GameLog::plain("launcher", "Started", 0).formatted(),
            "1970-01-01 00:00:00 UTC [INFO] [SyncMC] Started"
        );
    }

    #[test]
    fn malformed_and_incomplete_xml_remains_available() {
        let mut parser = LogParser::new("stdout");
        let malformed =
            "<log4j:Event><log4j:Message>useful &unknown; text</log4j:Message></log4j:Event>";
        assert_eq!(parser.line(malformed, 1)[0].line, malformed);
        assert!(parser.line(OPEN, 1).is_empty());
        let next = "<log4j:Event><log4j:Message>Recovered</log4j:Message></log4j:Event>";
        let entries = parser.line(next, 1);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].line, OPEN);
        assert_eq!(entries[1].line, "Recovered");
        parser.line(OPEN, 1);
        parser.line("<log4j:Message><![CDATA[incomplete", 1);
        assert_eq!(
            parser.finish(1)[0].line,
            format!("{OPEN}\n<log4j:Message><![CDATA[incomplete")
        );
    }

    #[test]
    fn invalid_xml_timestamp_uses_receipt_time_without_dropping_the_message() {
        for timestamp in ["18446744073709551615", "not-a-number"] {
            let raw = format!("<log4j:Event timestamp=\"{timestamp}\"><log4j:Message>Still readable</log4j:Message></log4j:Event>");
            let entry = LogParser::new("stdout").line(&raw, 15).remove(0);
            assert_eq!(entry.timestamp, 15);
            assert_eq!(entry.line, "Still readable");
            assert!(entry.formatted().starts_with("1970-01-01 00:00:15 UTC"));
        }
    }

    #[test]
    fn oversized_events_are_bounded_and_following_output_recovers() {
        let mut parser = LogParser::new("stdout");
        parser.line(OPEN, 1);
        parser.line("<log4j:Message><![CDATA[", 1);
        let long = "x".repeat(MAX_LINE_BYTES);
        let mut entries = Vec::new();
        for _ in 0..5 {
            entries.extend(parser.line(&long, 1));
        }
        assert_eq!(entries.len(), 1);
        assert!(entries[0].line.contains("ausgelassen"));
        assert_eq!(parser.xml.as_ref().unwrap().raw.len(), 0);
        assert!(parser
            .line("]]></log4j:Message></log4j:Event>", 1)
            .is_empty());
        assert_eq!(parser.line("still here", 1)[0].line, "still here");
    }

    #[test]
    fn oversized_unclosed_cdata_recovers_at_a_fresh_event() {
        let mut parser = LogParser::new("stdout");
        parser.line(OPEN, 1);
        parser.line("<log4j:Message><![CDATA[", 1);
        for _ in 0..5 {
            parser.line(&"x".repeat(MAX_LINE_BYTES), 1);
        }
        let entries = parser.line(
            "<log4j:Event><log4j:Message>Recovered</log4j:Message></log4j:Event>",
            1,
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].line, "Recovered");
    }

    #[test]
    fn continuation_formatting_grows_linearly_with_untrusted_metadata() {
        let mut entry = GameLog::plain("stdout", &"\n".repeat(MAX_LINE_BYTES), 0);
        entry.logger = Some("x".repeat(MAX_LINE_BYTES));
        assert!(entry.formatted().len() < 7 * MAX_LINE_BYTES);
    }
}
