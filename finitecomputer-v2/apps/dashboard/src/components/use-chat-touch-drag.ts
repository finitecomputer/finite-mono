"use client";

import { useEffect, useRef } from "react";
import type { SidebarChat } from "@/lib/hosted-web-chat-topics";

// A hold starts dragging; moving before the hold remains normal sidebar scrolling.
// Native non-passive move/end listeners are needed to cancel the browser gesture
// only after pickup. React's delegated touch listeners are passive.
export function useChatTouchDrag(options: {
  findChat: (row: HTMLElement) => SidebarChat | undefined;
  start: (chat: SidebarChat) => void;
  hover: (target: Element | null) => void;
  drop: (chat: SidebarChat, target: Element | null) => void;
  cancel: () => void;
}) {
  const navRef = useRef<HTMLElement>(null);
  const latest = useRef(options);
  useEffect(() => { latest.current = options; }, [options]);
  useEffect(() => {
    const nav = navRef.current;
    if (!nav) return;
    let frame = 0;
    let gesture: { chat: SidebarChat; x: number; y: number; active: boolean; timer: ReturnType<typeof setTimeout> } | null = null;
    function scrollAtEdge() {
      if (!gesture?.active || !nav) return;
      const bounds = nav.getBoundingClientRect();
      if (gesture.x >= bounds.left && gesture.x <= bounds.right && gesture.y >= bounds.top && gesture.y <= bounds.bottom) {
        const direction = gesture.y < bounds.top + 28 ? -1 : gesture.y > bounds.bottom - 28 ? 1 : 0;
        if (direction) {
          nav.scrollTop += direction * 8;
          latest.current.hover(document.elementFromPoint(gesture.x, gesture.y));
        }
      }
      frame = requestAnimationFrame(scrollAtEdge);
    }
    function cancel() {
      cancelAnimationFrame(frame);
      if (!gesture) return;
      clearTimeout(gesture.timer);
      gesture = null;
      latest.current.cancel();
    }
    function start(event: TouchEvent) {
      cancel();
      if (event.touches.length !== 1 || !(event.target instanceof Element)) return;
      const row = event.target.closest<HTMLElement>('[data-chat-id][draggable="true"]');
      if (!row || !event.target.closest(".finite-chat__thread-open")) return;
      const chat = latest.current.findChat(row);
      if (!chat) return;
      const touch = event.touches[0];
      const pending = {
        chat, x: touch.clientX, y: touch.clientY, active: false,
        timer: setTimeout(() => {
          if (gesture !== pending) return;
          pending.active = true;
          latest.current.start(chat);
          frame = requestAnimationFrame(scrollAtEdge);
        }, 350),
      };
      gesture = pending;
    }
    function move(event: TouchEvent) {
      if (!gesture) return;
      if (event.touches.length !== 1) { cancel(); return; }
      const touch = event.touches[0];
      if (!gesture.active) {
        if (Math.hypot(touch.clientX - gesture.x, touch.clientY - gesture.y) > 8) cancel();
        return;
      }
      event.preventDefault();
      gesture.x = touch.clientX;
      gesture.y = touch.clientY;
      latest.current.hover(document.elementFromPoint(touch.clientX, touch.clientY));
    }
    function end(event: TouchEvent) {
      if (!gesture) return;
      const current = gesture;
      clearTimeout(current.timer);
      cancelAnimationFrame(frame);
      gesture = null;
      if (!current.active) return; // Preserve a normal tap/click.
      event.preventDefault();
      const touch = event.changedTouches[0];
      latest.current.drop(current.chat, touch ? document.elementFromPoint(touch.clientX, touch.clientY) : null);
    }
    function contextMenu(event: Event) {
      if (gesture?.active) event.preventDefault();
    }
    function keyDown(event: KeyboardEvent) {
      if (event.key === "Escape") cancel();
    }
    nav.addEventListener("touchstart", start, { passive: true });
    nav.addEventListener("touchmove", move, { passive: false });
    nav.addEventListener("touchend", end, { passive: false });
    nav.addEventListener("touchcancel", cancel);
    nav.addEventListener("contextmenu", contextMenu);
    document.addEventListener("keydown", keyDown);
    return () => {
      cancelAnimationFrame(frame);
      if (gesture) clearTimeout(gesture.timer);
      nav.removeEventListener("touchstart", start);
      nav.removeEventListener("touchmove", move);
      nav.removeEventListener("touchend", end);
      nav.removeEventListener("touchcancel", cancel);
      nav.removeEventListener("contextmenu", contextMenu);
      document.removeEventListener("keydown", keyDown);
    };
  }, []);
  return navRef;
}
