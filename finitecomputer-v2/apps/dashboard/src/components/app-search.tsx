"use client";

import { useCallback, useEffect, useRef } from "react";
import {
	InputGroup,
	InputGroupAddon,
	InputGroupInput,
} from "@/components/ui/input-group";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { SearchIcon } from "lucide-react";

export function AppSearch() {
	const groupRef = useRef<HTMLDivElement>(null);

	const focusInput = useCallback(() => {
		const input = groupRef.current?.querySelector<HTMLInputElement>(
			"[data-slot=input-group-control]"
		);
		input?.focus({ preventScroll: true });
	}, []);

	useEffect(() => {
		const onKeyDown = (e: KeyboardEvent) => {
			if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				focusInput();
			}
		};
		document.addEventListener("keydown", onKeyDown);
		return () => document.removeEventListener("keydown", onKeyDown);
	}, [focusInput]);

	return (
		<InputGroup ref={groupRef}>
			<InputGroupAddon align="inline-start" className="pl-1.75">
				<SearchIcon
				/>
			</InputGroupAddon>
			<InputGroupInput
				aria-label="Search"
				name="q"
				placeholder="Search..."
				type="search"
			/>
			<InputGroupAddon align="inline-end">
				<KbdGroup>
					<Kbd>⌘</Kbd>
					<Kbd>K</Kbd>
				</KbdGroup>
			</InputGroupAddon>
		</InputGroup>
	);
}
