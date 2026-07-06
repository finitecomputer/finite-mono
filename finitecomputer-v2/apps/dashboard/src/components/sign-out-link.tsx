"use client";

import type { ComponentPropsWithoutRef } from "react";
import { forwardRef } from "react";

import { clearFiniteBrowserSessionState } from "@/lib/browser-session";

export const SignOutLink = forwardRef<HTMLAnchorElement, ComponentPropsWithoutRef<"a">>(
  function SignOutLink({ href = "/logout", onClick, ...props }, ref) {
    return (
      <a
        ref={ref}
        href={href}
        onClick={(event) => {
          clearFiniteBrowserSessionState();
          onClick?.(event);
        }}
        {...props}
      />
    );
  }
);
