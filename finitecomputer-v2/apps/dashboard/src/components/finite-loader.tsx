import type { CSSProperties, HTMLAttributes } from "react";

import { cn } from "@/lib/utils";

import styles from "./finite-loader.module.css";

const FINITE_LOGO_ROWS = 14;

export type FiniteLoaderVariant =
  | "rise"
  | "left"
  | "right"
  | "split"
  | "scan-up"
  | "scan-down"
  | "center-out"
  | "edges-in"
  | "alternate"
  | "signal"
  | "line-cycle"
  | "wipe-right"
  | "grow-middle";

export interface FiniteLoaderProps extends Omit<HTMLAttributes<HTMLSpanElement>, "children"> {
  label?: string;
  size?: number | string;
  variant?: FiniteLoaderVariant;
}

type LoaderBarStyle = CSSProperties & {
  "--order": number;
};

type LoaderStyle = CSSProperties & {
  "--finite-loader-size": string;
};

function rowClipPath(row: number) {
  const top = (row / FINITE_LOGO_ROWS) * 100;
  const bottom = ((FINITE_LOGO_ROWS - row - 1) / FINITE_LOGO_ROWS) * 100;
  return `inset(${top}% 0 ${bottom}% 0)`;
}

function rowOrder(variant: FiniteLoaderVariant, row: number) {
  const centerDistance = Math.abs(row - Math.floor(FINITE_LOGO_ROWS / 2));

  switch (variant) {
    case "rise":
    case "scan-up":
      return FINITE_LOGO_ROWS - row - 1;
    case "center-out":
      return centerDistance;
    case "edges-in":
      return Math.floor(FINITE_LOGO_ROWS / 2) - centerDistance;
    case "alternate":
      return row % 2 === 0 ? 0 : Math.floor(FINITE_LOGO_ROWS / 2);
    case "signal":
      return (row * 7) % FINITE_LOGO_ROWS;
    default:
      return row;
  }
}

export function FiniteLoader({
  className,
  label = "Loading",
  size = 72,
  style,
  variant = "rise",
  ...props
}: FiniteLoaderProps) {
  const isWholeIconReveal = variant === "wipe-right" || variant === "grow-middle";
  const loaderStyle: LoaderStyle = {
    "--finite-loader-size": typeof size === "number" ? `${size}px` : size,
    ...style,
  };

  return (
    <span
      aria-label={label}
      className={cn(styles.loader, className)}
      data-variant={variant}
      role="status"
      style={loaderStyle}
      {...props}
    >
      <span aria-hidden="true" className={styles.ghost} />
      {isWholeIconReveal ? (
        <span aria-hidden="true" className={styles.reveal} />
      ) : (
        Array.from({ length: FINITE_LOGO_ROWS }, (_, row) => {
          const barStyle: LoaderBarStyle = {
            "--order": rowOrder(variant, row),
            clipPath: rowClipPath(row),
          };

          return <span aria-hidden="true" className={styles.bar} key={row} style={barStyle} />;
        })
      )}
    </span>
  );
}
