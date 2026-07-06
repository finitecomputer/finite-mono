"use client";

import { LogoIcon } from "@/components/logo";
import { Button } from "@/components/ui/button";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarGroup,
	SidebarGroupLabel,
	SidebarHeader,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarRail,
} from "@/components/ui/sidebar";
import { AppSearch } from "@/components/app-search";
import { CustomTrigger } from "@/components/custom-trigger";
import { LatestChange } from "@/components/leatest-change";
import { LayoutDashboardIcon, MousePointerClickIcon, FunnelIcon, RepeatIcon, GitBranchIcon, UsersIcon, ChartPieIcon, UserIcon, PlugIcon, SettingsIcon } from "lucide-react";

export type SidebarNavItem = {
	title: string;
	url: string;
	icon: React.ReactNode;
	isActive?: boolean;
};

type SidebarSection = {
	label: string;
	items: SidebarNavItem[];
};

const navSections: SidebarSection[] = [
	{
		label: "Explore",
		items: [
			{
				title: "Dashboard",
				url: "#",
				icon: (
					<LayoutDashboardIcon
					/>
				),
				isActive: true,
			},
			{
				title: "Events",
				url: "#",
				icon: (
					<MousePointerClickIcon
					/>
				),
			},
			{
				title: "Funnels",
				url: "#",
				icon: (
					<FunnelIcon
					/>
				),
			},
			{
				title: "Retention",
				url: "#",
				icon: (
					<RepeatIcon
					/>
				),
			},
			{
				title: "Flows",
				url: "#",
				icon: (
					<GitBranchIcon
					/>
				),
			},
		],
	},
	{
		label: "Audiences",
		items: [
			{
				title: "Segments",
				url: "#",
				icon: (
					<UsersIcon
					/>
				),
			},
			{
				title: "Cohorts",
				url: "#",
				icon: (
					<ChartPieIcon
					/>
				),
			},
			{
				title: "Profiles",
				url: "#",
				icon: (
					<UserIcon
					/>
				),
			},
		],
	},
	{
		label: "Configure",
		items: [
			{
				title: "Integrations",
				url: "#",
				icon: (
					<PlugIcon
					/>
				),
			},
		],
	},
];

export function AppSidebar() {
	return (
		<Sidebar collapsible="icon" variant="sidebar">
			<SidebarHeader className="flex-row items-center justify-between">
				<Button asChild variant="ghost">
					<a href="#link">
						<LogoIcon />
						<span className="font-medium">Efferd</span>
					</a>
				</Button>
				<CustomTrigger place="sidebar" />
			</SidebarHeader>
			<SidebarContent>
				<SidebarGroup>
					<AppSearch />
				</SidebarGroup>
				{navSections.map((section) => (
					<SidebarGroup key={section.label}>
						<SidebarGroupLabel className="group-data-[collapsible=icon]:pointer-events-none">
							{section.label}
						</SidebarGroupLabel>
						<SidebarMenu>
							{section.items.map((item) => (
								<SidebarMenuItem key={item.title}>
									<SidebarMenuButton
										asChild
										isActive={item.isActive}
										tooltip={item.title}
									>
										<a href={item.url}>
											{item.icon}
											<span>{item.title}</span>
										</a>
									</SidebarMenuButton>
								</SidebarMenuItem>
							))}
						</SidebarMenu>
					</SidebarGroup>
				))}
			</SidebarContent>
			<SidebarFooter className="px-4">
				<LatestChange />
				<div className="flex items-center pt-4 pb-2">
					<Button
						asChild
						className="text-muted-foreground"
						size="icon-sm"
						variant="ghost"
					>
						<a aria-label="Settings" href="#">
							<SettingsIcon
							/>
						</a>
					</Button>
				</div>
			</SidebarFooter>
			<SidebarRail />
		</Sidebar>
	);
}
