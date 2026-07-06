import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";

import { CustomTrigger } from "@/components/custom-trigger";
import { NavUser } from "@/components/nav-user";
import { HelpCircleIcon, BellIcon } from "lucide-react";

export function AppNavbar() {
	return (
		<header className="sticky top-0 z-50 flex h-12 border-b">
			<div className="flex w-full shrink-0 items-center justify-between gap-2 bg-sidebar px-4 md:px-6">
				<div className="flex items-center gap-3">
					<CustomTrigger place="navbar" />
				</div>
				<p className="font-medium text-sm">Dashboard</p>
				<div className="flex items-center gap-3">
					<Button size="icon-sm" variant="outline">
						<HelpCircleIcon
						/>
					</Button>
					<Button aria-label="Notifications" size="icon-sm" variant="outline">
						<BellIcon
						/>
					</Button>
					<Separator
						className="h-4 data-[orientation=vertical]:self-center"
						orientation="vertical"
					/>
					<NavUser />
				</div>
			</div>
		</header>
	);
}
