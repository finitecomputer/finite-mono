"use client";

import * as React from "react";
import { LoaderCircleIcon } from "lucide-react";
import { useFormStatus } from "react-dom";

import { Button } from "@/components/ui/button";

type Props = React.ComponentProps<typeof Button> & {
  pendingLabel?: React.ReactNode;
};

export function FormActionButton({
  pendingLabel,
  children,
  disabled,
  ...props
}: Props) {
  const { pending } = useFormStatus();

  return (
    <Button aria-busy={pending} disabled={disabled || pending} {...props}>
      {pending ? (
        <>
          <LoaderCircleIcon className="animate-spin" />
          {pendingLabel ?? children}
        </>
      ) : (
        children
      )}
    </Button>
  );
}
