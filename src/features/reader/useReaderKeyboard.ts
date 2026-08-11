import { onMounted, onUnmounted, type Ref } from 'vue';

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable
    || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
    || Boolean(target.closest('.xterm, [role="textbox"]'));
}

export function shouldInterceptFind(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f' && !event.altKey && !isEditableTarget(event.target);
}

export interface ReaderKeyboardOptions {
  input: Ref<HTMLInputElement | null>;
  onOpen: () => void;
  onNext: () => void;
  onPrevious: () => void;
  onClose: () => void;
}

export function useReaderKeyboard(options: ReaderKeyboardOptions) {
  function focusInput() {
    options.onOpen();
    window.requestAnimationFrame(() => {
      options.input.value?.focus();
      if (!options.input.value) document.querySelector<HTMLInputElement>('.session-search-controls input')?.focus();
    });
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    if (shouldInterceptFind(event)) {
      event.preventDefault();
      focusInput();
    }
  }

  function onSearchKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Enter') {
      event.preventDefault();
      if (event.shiftKey) options.onPrevious();
      else options.onNext();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      options.onClose();
    }
  }

  onMounted(() => window.addEventListener('keydown', onKeydown));
  onUnmounted(() => window.removeEventListener('keydown', onKeydown));
  return { onSearchKeydown, focusInput };
}
