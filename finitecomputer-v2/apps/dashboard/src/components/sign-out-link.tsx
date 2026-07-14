"use client";

import type { ComponentPropsWithoutRef } from "react";
import { forwardRef } from "react";

import { clearFiniteBrowserSessionState } from "@/lib/browser-session";
import { endEmbeddedBrainSession } from "@/lib/brain-session-bridge";

export const SignOutLink = forwardRef<HTMLAnchorElement, ComponentPropsWithoutRef<"a">>(
  function SignOutLink({ href = "/logout", onClick, ...props }, ref) {
    return (
      <a
        ref={ref}
        href={href}
        onClick={(event) => {
          endEmbeddedBrainSession();
          clearFiniteBrowserSessionState();
          onClick?.(event);
        }}
        {...props}
      />
    );
  }
);
