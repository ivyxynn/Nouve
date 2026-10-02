// The card shell shared by every view.
//
// It lives in its own module rather than in views.ts because the chat view needs
// it too, and views.ts already imports the chat builder — keeping `card` here is
// what stops that pair from becoming an import cycle.

import { h } from "./dom";
import { washRGBA, type Wash } from "../core/layout";

export function card(wash: Wash, ...children: (Node | string)[]): HTMLElement {
  const el = h("div", { class: "card" }, ...children);
  applyWash(el, wash);
  return el;
}

/**
 * Sets — or clears — a card's wash. Kept separate from `card()` because the
 * approval card changes colour between renders: the same element is amber for a
 * routine permission and red when the file or command is dangerous.
 *
 * The overview calls this on every sync, so the current value is remembered and
 * an unchanged wash is skipped rather than re-written to the style attribute.
 */
export function applyWash(el: HTMLElement, wash: Wash) {
  if (el.dataset.wash === String(wash)) return;
  el.dataset.wash = String(wash);
  if (wash) {
    el.classList.add("wash");
    el.style.setProperty("--wash", washRGBA(wash));
  } else {
    el.classList.remove("wash");
    el.style.removeProperty("--wash");
  }
}
