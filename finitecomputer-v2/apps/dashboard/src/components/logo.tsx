import type React from "react";

const finiteLogoMask = {
	backgroundColor: "currentColor",
	display: "inline-block",
	flexShrink: 0,
	height: "1em",
	mask: 'url("/finite-logo.svg") center / contain no-repeat',
	WebkitMask: 'url("/finite-logo.svg") center / contain no-repeat',
	width: "1em",
} satisfies React.CSSProperties;

export const LogoIcon = ({
	style,
	...props
}: React.ComponentProps<"span">) => (
	<span
		aria-hidden="true"
		style={{ ...finiteLogoMask, ...style }}
		{...props}
	/>
);

export const Logo = ({
	style,
	...props
}: React.ComponentProps<"span">) => (
	<span
		aria-hidden="true"
		style={{ ...finiteLogoMask, ...style }}
		{...props}
	/>
);
