"use client";

import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { ArrowDownIcon, ArrowUpIcon, ArrowUpDownIcon } from "lucide-react";
import type { AdminTableSort } from "@/lib/admin-users-table";
import styles from "@/styles/admin-users-table.module.css";

/** Keep the header in the page's vertical scroll flow and sync horizontal scroll. */
export function AdminTableScroll({ children, sort, onSort }: { children: ReactNode; sort: AdminTableSort; onSort: (column: string) => void }) {
  const bodyRef = useRef<HTMLDivElement>(null);
  const headerRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ x: number; scrollLeft: number } | null>(null);
  const suppressClick = useRef(false);
  const [dragging, setDragging] = useState(false);
  const [columns, setColumns] = useState<{ label: string; width: number }[]>([]);
  const [headerHeight, setHeaderHeight] = useState(0);
  const [edges, setEdges] = useState({ left: false, right: false });

  function updateEdges(surface: HTMLDivElement) {
    const left = surface.scrollLeft > 1;
    const right = surface.scrollLeft < surface.scrollWidth - surface.clientWidth - 1;
    setEdges((previous) => previous.left === left && previous.right === right ? previous : { left, right });
  }

  function moveByColumn(event: KeyboardEvent<HTMLDivElement>) {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    // Dialog events can bubble through portals; only handle this scroll surface.
    if (!event.currentTarget.contains(event.target as Node)) return;
    if (event.currentTarget === bodyRef.current && event.target !== event.currentTarget && !(event.target instanceof HTMLElement && event.target.matches("tr[tabindex]"))) return;
    const body = bodyRef.current;
    if (!body) return;
    event.preventDefault();
    let offset = 0;
    const boundaries = columns.map((column) => {
      const start = offset;
      offset += column.width;
      return start;
    });
    const current = body.scrollLeft;
    const maximum = Math.max(0, body.scrollWidth - body.clientWidth);
    const next = event.key === "ArrowRight"
      ? boundaries.find((boundary) => boundary > current + 1) ?? maximum
      : [...boundaries].reverse().find((boundary) => boundary < current - 1) ?? 0;
    body.scrollLeft = Math.min(maximum, Math.max(0, next));
    if (headerRef.current) headerRef.current.scrollLeft = body.scrollLeft;
  }

  useLayoutEffect(() => {
    const table = bodyRef.current?.querySelector("table");
    const head = table?.tHead;
    if (!table || !head) return;
    const measure = () => {
      const next = Array.from(head.rows[0].cells, (cell) => ({
        label: cell.textContent ?? "",
        width: cell.getBoundingClientRect().width,
      }));
      setColumns((previous) => JSON.stringify(previous) === JSON.stringify(next) ? previous : next);
      setHeaderHeight(head.getBoundingClientRect().height);
      if (bodyRef.current) updateEdges(bodyRef.current);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(table);
    observer.observe(head);
    if (bodyRef.current) observer.observe(bodyRef.current);
    return () => observer.disconnect();
  }, [children]);

  return (
    <div className={styles.frame} data-fade-left={edges.left} data-fade-right={edges.right}>
      <div
        ref={headerRef}
        className={styles.stickyHeader}
        data-dragging={dragging}
        role="region"
        aria-label="Table column headers. Drag to scroll, or use Left and Right arrow keys to move one column."
        tabIndex={0}
        onKeyDown={moveByColumn}
        onScroll={(event) => {
          if (bodyRef.current) bodyRef.current.scrollLeft = event.currentTarget.scrollLeft;
        }}
        onPointerDown={(event) => {
          if (event.button !== 0 || event.pointerType !== "mouse") return;
          drag.current = { x: event.clientX, scrollLeft: event.currentTarget.scrollLeft };
          suppressClick.current = false;
        }}
        onPointerMove={(event) => {
          if (!drag.current) return;
          const distance = drag.current.x - event.clientX;
          if (!suppressClick.current && Math.abs(distance) < 5) return;
          suppressClick.current = true;
          if (!event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.setPointerCapture(event.pointerId);
          setDragging(true);
          event.currentTarget.scrollLeft = drag.current.scrollLeft + distance;
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
          drag.current = null;
          setDragging(false);
        }}
        onClickCapture={(event) => {
          if (suppressClick.current && event.detail !== 0) {
            event.preventDefault();
            event.stopPropagation();
            suppressClick.current = false;
          }
        }}
        onPointerCancel={() => { drag.current = null; setDragging(false); }}
        onLostPointerCapture={() => { drag.current = null; setDragging(false); }}
      >
        <div className={styles.headerColumns} style={{ width: columns.reduce((sum, column) => sum + column.width, 0) }}>
          {columns.map((column) => {
            const selected = sort.column === column.label;
            const Icon = selected ? sort.direction === "ascending" ? ArrowUpIcon : ArrowDownIcon : ArrowUpDownIcon;
            return <button key={column.label} type="button" style={{ width: column.width }}
              aria-label={`Sort by ${column.label}, ${selected && sort.direction === "ascending" ? "descending" : "ascending"}`}
              onClick={() => onSort(column.label)}>
              {column.label}<Icon aria-hidden="true" className="size-3.5 shrink-0" />
            </button>;
          })}
        </div>
      </div>
      <div
        ref={bodyRef}
        className={`${styles.scroll} ${styles.tableBody}`}
        role="region"
        aria-label="Users table, scroll for more columns"
        tabIndex={0}
        onKeyDown={moveByColumn}
        onClick={(event) => {
          const target = event.target as HTMLElement;
          if (event.currentTarget.contains(target) && !target.closest("button, a, input, select, textarea, summary, tr[tabindex], [contenteditable=true]")) {
            event.currentTarget.focus({ preventScroll: true });
          }
        }}
        onScroll={(event) => {
          if (headerRef.current) headerRef.current.scrollLeft = event.currentTarget.scrollLeft;
          updateEdges(event.currentTarget);
        }}
      >
        <div style={{ marginTop: -headerHeight }}>{children}</div>
      </div>
    </div>
  );
}
