import {
  AfterViewInit,
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  forwardRef,
  Input,
  OnDestroy,
  signal,
  ViewChild,
} from '@angular/core';
import { ControlValueAccessor, NG_VALUE_ACCESSOR } from '@angular/forms';

export interface SelectOption {
  value: string;
  label: string;
  description?: string;
  image?: string;
  icon?: string;
}

let nextSelectId = 0;

/** Select-only by default; searchable fields focus an editable combobox in the popover. */
@Component({
  selector: 'sync-select',
  standalone: true,
  templateUrl: './glass-select.component.html',
  styleUrl: './glass-select.component.scss',
  changeDetection: ChangeDetectionStrategy.OnPush,
  providers: [{
    provide: NG_VALUE_ACCESSOR,
    useExisting: forwardRef(() => GlassSelect),
    multi: true,
  }],
})
export class GlassSelect implements ControlValueAccessor, AfterViewInit, OnDestroy {
  @Input() label = 'Auswahl';
  @Input() appearance: 'field' | 'compact' | 'instance' = 'field';
  @Input() placeholder = 'Auswählen';
  @Input() searchable = false;
  @Input() searchPlaceholder = 'Suchen …';

  @Input()
  set options(options: SelectOption[]) {
    this.items = options ?? [];
    if (this.open()) {
      this.activeIndex.set(this.selectedIndex >= 0 ? this.selectedIndex : 0);
      if (!this.items.length) this.closeMenu();
      else queueMicrotask(() => { this.positionPanel(); this.scrollActiveIntoView(); });
    }
  }
  get options(): SelectOption[] { return this.items; }

  @ViewChild('trigger', { static: true }) private trigger!: ElementRef<HTMLButtonElement>;
  @ViewChild('panel', { static: true }) private panel!: ElementRef<HTMLDivElement>;
  @ViewChild('optionList', { static: true }) private optionList!: ElementRef<HTMLDivElement>;
  @ViewChild('searchInput') private searchInput?: ElementRef<HTMLInputElement>;

  readonly listboxId = `sync-select-${++nextSelectId}-listbox`;
  readonly panelId = `${this.listboxId}-panel`;
  readonly searchQuery = signal('');
  readonly open = signal(false);
  readonly disabled = signal(false);
  readonly value = signal('');
  readonly activeIndex = signal(0);
  readonly panelLeft = signal(0);
  readonly panelTop = signal(0);
  readonly panelWidth = signal(240);
  readonly panelHeight = signal(360);
  readonly opensAbove = signal(false);

  private items: SelectOption[] = [];
  private resizeObserver?: ResizeObserver;
  private typeahead = '';
  private typeaheadTimer?: ReturnType<typeof setTimeout>;
  private destroyed = false;
  private onChange: (value: string) => void = () => {};
  private onTouched: () => void = () => {};
  private readonly onViewportChange = () => { if (this.open()) this.positionPanel(); };

  get selectedOption(): SelectOption | undefined {
    return this.items.find((option) => option.value === this.value());
  }

  get visibleOptions(): SelectOption[] {
    const query = this.searchQuery().trim().toLocaleLowerCase();
    return !this.searchable || !query ? this.items : this.items.filter(option =>
      `${option.label} ${option.description ?? ''}`.toLocaleLowerCase().includes(query));
  }

  private get selectedIndex(): number {
    return this.visibleOptions.findIndex((option) => option.value === this.value());
  }

  ngAfterViewInit(): void {
    window.addEventListener('resize', this.onViewportChange);
    window.addEventListener('scroll', this.onViewportChange, true);
    window.visualViewport?.addEventListener('resize', this.onViewportChange);
    window.visualViewport?.addEventListener('scroll', this.onViewportChange);
    this.resizeObserver = new ResizeObserver(this.onViewportChange);
    this.resizeObserver.observe(this.trigger.nativeElement);
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    clearTimeout(this.typeaheadTimer);
    this.resizeObserver?.disconnect();
    window.removeEventListener('resize', this.onViewportChange);
    window.removeEventListener('scroll', this.onViewportChange, true);
    window.visualViewport?.removeEventListener('resize', this.onViewportChange);
    window.visualViewport?.removeEventListener('scroll', this.onViewportChange);
    if (this.panel.nativeElement.matches(':popover-open')) this.panel.nativeElement.hidePopover();
  }

  writeValue(value: unknown): void {
    this.value.set(typeof value === 'string' ? value : '');
    if (this.open()) {
      this.activeIndex.set(Math.max(0, this.selectedIndex));
      this.scrollActiveIntoView();
    }
  }

  registerOnChange(fn: (value: string) => void): void { this.onChange = fn; }
  registerOnTouched(fn: () => void): void { this.onTouched = fn; }
  setDisabledState(disabled: boolean): void {
    this.disabled.set(disabled);
    if (disabled) this.closeMenu();
  }

  toggleMenu(event: MouseEvent): void {
    // The declarative target establishes the invoker relationship; CVA owns selection/open state.
    event.preventDefault();
    event.stopPropagation();
    if (this.open()) this.closeMenu();
    else this.openMenu();
  }

  onToggle(event: Event): void {
    const isOpen = (event as ToggleEvent).newState === 'open';
    this.open.set(isOpen);
    if (!isOpen) {
      this.resetTypeahead();
      this.onTouched();
    }
  }

  onBlur(): void {
    if (!this.open()) this.onTouched();
  }

  onSearch(event: Event): void {
    this.searchQuery.set((event.target as HTMLInputElement).value);
    this.activeIndex.set(0);
    this.optionList.nativeElement.scrollTop = 0;
  }

  onKeydown(event: KeyboardEvent): void {
    if (this.disabled()) return;
    const editingSearch = event.target === this.searchInput?.nativeElement;
    if (event.key === 'Escape') {
      if (this.open()) {
        // Prevent the same Escape from dismissing an enclosing creation/version modal.
        event.preventDefault();
        event.stopPropagation();
        this.closeMenu(true);
      }
      return;
    }
    if (event.key === 'Tab') {
      if (this.open()) this.closeMenu(editingSearch);
      return; // Let browser focus order and an enclosing modal focus trap handle Tab.
    }
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    // Preserve ordinary text editing, including spaces and Home/End, in the search field.
    if (editingSearch && !['ArrowDown', 'ArrowUp', 'Enter'].includes(event.key)) return;
    if (['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', ' '].includes(event.key)) {
      event.preventDefault();
      event.stopPropagation();
      const wasOpen = this.open();
      if (event.key === 'Enter' || event.key === ' ') {
        if (wasOpen) this.choose(this.activeIndex());
        else this.openMenu();
        return;
      }
      if (!wasOpen) this.openMenu(event.key === 'ArrowUp' || event.key === 'End' ? 'last' : 'first');
      if (!this.open()) return;
      const count = this.visibleOptions.length;
      if (!count) return;
      if (event.key === 'Home') this.activeIndex.set(0);
      else if (event.key === 'End') this.activeIndex.set(count - 1);
      else if (wasOpen) {
        const direction = event.key === 'ArrowDown' ? 1 : -1;
        this.activeIndex.set((this.activeIndex() + direction + count) % count);
      }
      this.scrollActiveIntoView();
      return;
    }
    if (event.key.length === 1 && /\S/u.test(event.key)) {
      event.preventDefault();
      event.stopPropagation();
      if (!this.open()) this.openMenu();
      if (this.searchable) {
        this.searchQuery.set(event.key);
        this.activeIndex.set(0);
        this.searchInput?.nativeElement.focus({ preventScroll: true });
      } else this.findTypedOption(event.key);
    }
  }

  choose(index: number, event?: MouseEvent): void {
    event?.stopPropagation();
    const option = this.visibleOptions[index];
    if (this.disabled() || !option) return;
    this.value.set(option.value);
    this.onChange(option.value);
    this.closeMenu(true);
    this.onTouched();
  }

  optionId(index: number): string { return `${this.listboxId}-option-${index}`; }

  iconClass(icon: string): string { return icon.startsWith('ph-') ? icon : `ph-${icon}`; }

  private openMenu(edge?: 'first' | 'last'): void {
    if (this.disabled() || !this.items.length || this.destroyed) return;
    this.searchQuery.set('');
    this.activeIndex.set(edge === 'first' && this.selectedIndex < 0 ? 0
      : edge === 'last' && this.selectedIndex < 0 ? this.items.length - 1
      : Math.max(0, this.selectedIndex));
    this.positionPanel();
    this.panel.nativeElement.showPopover({ source: this.trigger.nativeElement });
    this.open.set(true);
    (this.searchable ? this.searchInput?.nativeElement : this.trigger.nativeElement)?.focus({ preventScroll: true });
    // Wait for the active option class and aria-activedescendant to render before scrolling.
    queueMicrotask(() => this.scrollActiveIntoView());
  }

  private closeMenu(restoreFocus = false): void {
    if (!this.open()) return;
    this.open.set(false);
    this.resetTypeahead();
    if (this.panel.nativeElement.matches(':popover-open')) this.panel.nativeElement.hidePopover();
    this.onTouched();
    if (restoreFocus && !this.destroyed) this.trigger.nativeElement.focus({ preventScroll: true });
  }

  private positionPanel(): void {
    if (this.destroyed || !this.trigger) return;
    const rect = this.trigger.nativeElement.getBoundingClientRect();
    const viewport = window.visualViewport;
    const viewportLeft = viewport?.offsetLeft ?? 0;
    const viewportTop = viewport?.offsetTop ?? 0;
    const viewportWidth = viewport?.width ?? window.innerWidth;
    const viewportHeight = viewport?.height ?? window.innerHeight;
    const edge = 12;
    const gap = 7;
    const rowHeight = this.appearance === 'instance' ? 70
      : this.items.some((option) => option.description) ? 62
      : this.items.some((option) => option.image || option.icon) ? 56 : 48;
    const desiredHeight = Math.min(360, this.items.length * rowHeight + 14 + (this.searchable ? 52 : 0));
    const below = viewportTop + viewportHeight - edge - rect.bottom - gap;
    const above = rect.top - viewportTop - edge - gap;
    const openAbove = below < desiredHeight && above > below;
    const availableHeight = Math.max(0, openAbove ? above : below);
    const height = Math.min(desiredHeight, availableHeight);
    const minWidth = this.appearance === 'instance' ? 300 : 240;
    const width = Math.min(Math.max(rect.width, minWidth), Math.max(0, viewportWidth - edge * 2));
    this.panelWidth.set(width);
    this.panelHeight.set(height);
    this.opensAbove.set(openAbove);
    this.panelLeft.set(Math.max(viewportLeft + edge, Math.min(rect.left, viewportLeft + viewportWidth - edge - width)));
    const top = openAbove ? rect.top - gap - height : rect.bottom + gap;
    this.panelTop.set(Math.max(viewportTop + edge, Math.min(top, viewportTop + viewportHeight - edge - height)));
  }

  private scrollActiveIntoView(): void {
    if (!this.open() || this.destroyed) return;
    const option = this.panel.nativeElement.querySelector<HTMLElement>(`#${this.optionId(this.activeIndex())}`);
    if (!option) return;
    // Scroll only this listbox, never the page or the modal behind it.
    const panel = this.optionList.nativeElement;
    const top = option.offsetTop;
    const bottom = top + option.offsetHeight;
    if (top < panel.scrollTop) panel.scrollTop = top;
    else if (bottom > panel.scrollTop + panel.clientHeight) panel.scrollTop = bottom - panel.clientHeight;
  }

  private findTypedOption(character: string): void {
    clearTimeout(this.typeaheadTimer);
    this.typeahead += character.toLocaleLowerCase();
    const repeatedCharacter = [...this.typeahead].every((letter) => letter === this.typeahead[0]);
    const query = repeatedCharacter ? this.typeahead[0] : this.typeahead;
    const start = repeatedCharacter ? this.activeIndex() + 1 : this.activeIndex();
    for (let offset = 0; offset < this.items.length; offset++) {
      const index = (start + offset) % this.items.length;
      if (this.items[index].label.toLocaleLowerCase().startsWith(query)) {
        this.activeIndex.set(index);
        this.scrollActiveIntoView();
        break;
      }
    }
    this.typeaheadTimer = setTimeout(() => { this.typeahead = ''; }, 700);
  }

  private resetTypeahead(): void {
    clearTimeout(this.typeaheadTimer);
    this.typeahead = '';
  }
}
