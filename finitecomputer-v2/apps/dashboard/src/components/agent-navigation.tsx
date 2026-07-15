"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  BotIcon,
  BrainIcon,
  Building2Icon,
  CreditCardIcon,
  Globe2Icon,
  LogOutIcon,
  MoreHorizontalIcon,
  PlugIcon,
  WrenchIcon,
  type LucideIcon,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export function AgentNavigation({
  machineId,
  onNavigate,
  showSkills,
}: {
  machineId: string;
  onNavigate?: () => void;
  showSkills: boolean;
}) {
  const pathname = usePathname() ?? "";
  const root = `/dashboard/machines/${encodeURIComponent(machineId)}`;
  const items: Array<{
    label: string;
    href?: string;
    icon: LucideIcon;
    active: boolean;
    note?: string;
  }> = [
    { label: "Agent", href: root, icon: BotIcon, active: pathname === root },
    {
      label: "Connections",
      href: `${root}/connections`,
      icon: PlugIcon,
      active: pathname === `${root}/connections`,
    },
    {
      label: "Sites",
      icon: Globe2Icon,
      active: false,
      note: "Open a site from Preview in chat",
    },
    {
      label: "Brain",
      href: `${root}/brain`,
      icon: BrainIcon,
      active: pathname === `${root}/brain`,
    },
    ...(showSkills
      ? [{
          label: "Skills",
          href: `/dashboard/skills?machine=${encodeURIComponent(machineId)}`,
          icon: WrenchIcon,
          active: pathname === "/dashboard/skills",
        }]
      : []),
  ];

  return (
    <nav className="finite-agent-nav" aria-label="Agent navigation">
      {items.map(({ active, href, icon: Icon, label, note }) =>
        href ? (
          <Link
            key={label}
            href={href}
            className={cn("finite-agent-nav__item", active && "is-active")}
            aria-current={active ? "page" : undefined}
            onClick={onNavigate}
          >
            <Icon className="size-4" />
            <span>{label}</span>
          </Link>
        ) : (
          <span
            key={label}
            className="finite-agent-nav__item is-disabled"
            aria-disabled="true"
            title={note}
          >
            <Icon className="size-4" />
            <span>{label}</span>
          </span>
        )
      )}
    </nav>
  );
}

export function AccountMenu({
  fallbackLabel = "Local development account",
  viewerEmail,
  side = "bottom",
}: {
  fallbackLabel?: string;
  viewerEmail?: string | null;
  side?: "top" | "bottom";
}) {
  const label = viewerEmail || fallbackLabel;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button type="button" className="finite-chat__user-row" aria-label="Account menu">
          <span className="finite-chat__avatar" aria-hidden>{initials(label)}</span>
          <span className="finite-chat__user-name">{label}</span>
          <MoreHorizontalIcon className="size-4" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        side={side}
        sideOffset={8}
        className="finite-chat__app-menu"
      >
        <DropdownMenuLabel className="finite-chat__app-menu-heading">Signed in as</DropdownMenuLabel>
        <div className="finite-chat__app-menu-account">
          <span className="finite-chat__avatar" aria-hidden>{initials(label)}</span>
          <span>{label}</span>
        </div>
        <DropdownMenuSeparator className="finite-chat__app-menu-separator" />
        <AccountMenuLink href="/dashboard" icon={Building2Icon} label="Account" note="Agents and account settings" />
        <AccountMenuLink href="/dashboard#billing" icon={CreditCardIcon} label="Billing" note="Plan and payment details" />
        <DropdownMenuSeparator className="finite-chat__app-menu-separator" />
        <DropdownMenuItem asChild className="finite-chat__app-menu-item">
          <Link href="/logout"><LogOutIcon /><span><strong>Sign out</strong><small>End this session</small></span></Link>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function AccountMenuLink({
  href,
  icon: Icon,
  label,
  note,
}: {
  href: string;
  icon: LucideIcon;
  label: string;
  note: string;
}) {
  return (
    <DropdownMenuItem asChild className="finite-chat__app-menu-item">
      <Link href={href}><Icon /><span><strong>{label}</strong><small>{note}</small></span></Link>
    </DropdownMenuItem>
  );
}

function initials(value: string) {
  const seed = value.includes("@") ? value.split("@")[0] : value;
  return seed
    .split(/[\s._-]+/u)
    .filter(Boolean)
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase() ?? "")
    .join("") || "F";
}
