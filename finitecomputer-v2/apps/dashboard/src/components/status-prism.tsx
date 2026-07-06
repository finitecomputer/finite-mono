"use client";

import { useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from "react";

import { cn } from "@/lib/utils";

export type CubeState = "stuck" | "working" | "happy" | "off";

type DragStart = {
  pointerX: number;
  pointerY: number;
  rotX: number;
  rotY: number;
};

export function StatusPrism({
  state = "working",
  size = "md",
  className,
}: {
  state?: CubeState;
  size?: "md" | "sm";
  className?: string;
}) {
  const [rotation, setRotation] = useState({ x: 0, y: 0 });
  const [isManual, setIsManual] = useState(false);
  const [isDragging, setIsDragging] = useState(false);
  const dragStart = useRef<DragStart | null>(null);

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    dragStart.current = {
      pointerX: event.clientX,
      pointerY: event.clientY,
      rotX: rotation.x,
      rotY: rotation.y,
    };
    setIsDragging(true);
    setIsManual(true);
  };

  const handlePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!dragStart.current) return;
    const dx = event.clientX - dragStart.current.pointerX;
    const dy = event.clientY - dragStart.current.pointerY;
    setRotation({
      x: dragStart.current.rotX - dy * 0.5,
      y: dragStart.current.rotY + dx * 0.5,
    });
  };

  const handlePointerEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    dragStart.current = null;
    setIsDragging(false);
  };

  const spinStyle: CSSProperties | undefined = isManual
    ? { transform: `rotateX(${rotation.x}deg) rotateY(${rotation.y}deg)` }
    : undefined;

  return (
    <div
      className={cn(
        "status-prism-scene",
        size === "md" ? "h-28 w-28" : "h-16 w-16",
        size === "sm" && "status-prism-scene--sm",
        `status-prism-scene--${state}`,
        isDragging ? "cursor-grabbing" : "cursor-grab",
        className
      )}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerEnd}
      onPointerCancel={handlePointerEnd}
    >
      <div className="status-prism-caustic" />
      <div className="status-prism-shadow" />
      <div className="status-cube-orbit">
        <div className={cn("status-cube-spin", isManual && "status-cube-spin--manual")} style={spinStyle}>
          <div className="status-cube">
            <span className="status-cube-face status-cube-face--front" />
            <span className="status-cube-face status-cube-face--back" />
            <span className="status-cube-face status-cube-face--right" />
            <span className="status-cube-face status-cube-face--left" />
            <span className="status-cube-face status-cube-face--top" />
            <span className="status-cube-face status-cube-face--bottom" />
          </div>
        </div>
      </div>
    </div>
  );
}
