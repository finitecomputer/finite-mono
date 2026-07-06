import Link from "next/link";

import { cn } from "@/lib/utils";

type FiniteBrandProps = {
  className?: string;
  href?: string;
};

const LABEL = "Finite.Computer";

export function FiniteBrand({ className, href }: FiniteBrandProps) {
  const classes = cn("ocean-brand", className);

  if (href) {
    return (
      <Link href={href} className={classes}>
        {LABEL}
      </Link>
    );
  }

  return <span className={classes}>{LABEL}</span>;
}
