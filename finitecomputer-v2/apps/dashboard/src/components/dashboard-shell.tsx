"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import {
  BotIcon,
  BrainIcon,
  ChevronRightIcon,
  LayoutDashboardIcon,
  Layers3Icon,
  LogOutIcon,
  MessageSquareIcon,
  PlugIcon,
  type LucideIcon,
} from "lucide-react";

import { FiniteBrand } from "@/components/finite-brand";
import { SignOutLink } from "@/components/sign-out-link";
import { cn } from "@/lib/utils";
import "@/styles/ocean-shell.css";

type MachineNavItem = {
  id: string;
  ownerLabel: string;
  siteUrl?: string;
};

type DashboardShellProps = {
  children: React.ReactNode;
  viewerEmail?: string | null;
  isAdmin: boolean;
  machines: MachineNavItem[];
  saasMode: boolean;
};

type SectionLink = {
  label: string;
  href: string;
  icon: LucideIcon;
  active: boolean;
  disabled?: boolean;
};

function activeMachineIdFromPath(pathname: string) {
  const match = pathname.match(/^\/dashboard\/machines\/([^/]+)/u);
  return match?.[1] ? decodeURIComponent(match[1]) : null;
}

function sectionLinks(
  pathname: string,
  machine: MachineNavItem | null,
  saasMode: boolean
): SectionLink[] {
  const machineHref = machine ? `/dashboard/machines/${machine.id}` : "/dashboard";
  const chatHref = machine ? `/dashboard/machines/${machine.id}/chat` : "/dashboard";
  const skillsHref = machine ? `/dashboard/skills?machine=${encodeURIComponent(machine.id)}` : "/dashboard/skills";

  if (saasMode) {
    return [
      {
        label: "Agent",
        href: machineHref,
        icon: BotIcon,
        active: pathname === "/dashboard" || (machine ? pathname === `/dashboard/machines/${machine.id}` : false),
      },
      {
        label: "Connections",
        href: machine ? `${machineHref}/connections` : "/dashboard",
        icon: PlugIcon,
        active: machine ? pathname === `${machineHref}/connections` : false,
        disabled: !machine,
      },
      {
        label: "Brain",
        href: machine ? `${machineHref}/brain` : "/dashboard",
        icon: BrainIcon,
        active: machine ? pathname === `${machineHref}/brain` : false,
        disabled: !machine,
      },
      {
        label: "Chat",
        href: chatHref,
        icon: MessageSquareIcon,
        active: machine ? pathname === `/dashboard/machines/${machine.id}/chat` : false,
        disabled: !machine,
      },
    ];
  }

  return [
    {
      label: "Agents",
      href: "/dashboard",
      icon: BotIcon,
      active: pathname === "/dashboard",
    },
    {
      label: "Overview",
      href: machineHref,
      icon: LayoutDashboardIcon,
      active: machine ? pathname === `/dashboard/machines/${machine.id}` : false,
      disabled: !machine,
    },
    {
      label: "Chat",
      href: chatHref,
      icon: MessageSquareIcon,
      active: machine ? pathname === `/dashboard/machines/${machine.id}/chat` : false,
      disabled: !machine,
    },
    {
      label: "Skills",
      href: skillsHref,
      icon: Layers3Icon,
      active: pathname === "/dashboard/skills",
    },
  ];
}

function MachineSwitcher({
  activeMachine,
  machines,
  onNavigate,
}: {
  activeMachine: MachineNavItem | null;
  machines: MachineNavItem[];
  onNavigate?: () => void;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div className="ocean-machine-switcher">
      <button
        type="button"
        className="ocean-machine-switcher__button"
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((value) => !value)}
      >
        <span className="ocean-machine-switcher__dot" aria-hidden />
        <span className="ocean-machine-switcher__label">
          {activeMachine?.ownerLabel ?? machines[0]?.ownerLabel ?? "No machines"}
        </span>
        <ChevronRightIcon className={cn("ocean-machine-switcher__chevron", open && "is-open")} />
      </button>

      {open ? (
        <div className="ocean-menu ocean-machine-switcher__menu" role="menu">
          {machines.map((machine) => (
            <Link
              key={machine.id}
              href={`/dashboard/machines/${machine.id}`}
              className={cn("ocean-menu-item", activeMachine?.id === machine.id && "is-active")}
              role="menuitem"
              onClick={() => {
                setOpen(false);
                onNavigate?.();
              }}
            >
              <BotIcon className="size-4" />
              <span>{machine.ownerLabel}</span>
            </Link>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function DashboardAppSection({
  children,
  activeMachine,
  machines,
  pathname,
  saasMode,
  showMachineFleet,
  viewerEmail,
}: {
  children: React.ReactNode;
  activeMachine: MachineNavItem | null;
  machines: MachineNavItem[];
  pathname: string;
  saasMode: boolean;
  showMachineFleet: boolean;
  viewerEmail?: string | null;
}) {
  const selectedMachine = activeMachine ?? machines[0] ?? null;
  const links = sectionLinks(pathname, selectedMachine, saasMode);
  const scrollRef = useRef<HTMLElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [pathname]);

  return (
    <div className="ocean-app-section">
      <header className="ocean-app-header">
        <div className="ocean-app-header__brand">
          <FiniteBrand
            href={selectedMachine ? `/dashboard/machines/${selectedMachine.id}` : "/dashboard"}
          />
        </div>

        <div className="ocean-app-header__center">
          {showMachineFleet ? <MachineSwitcher activeMachine={selectedMachine} machines={machines} /> : null}
          <nav
            className="ocean-section-tabs"
            aria-label="Dashboard section"
          >
            {links.map((item) => {
              const Icon = item.icon;
              const className = cn(
                "ocean-section-tab",
                item.active && "is-active",
                item.disabled && "is-disabled"
              );

              if (item.disabled) {
                return (
                  <span key={item.label} className={className} aria-disabled="true">
                    <Icon className="size-4" />
                    <span>{item.label}</span>
                  </span>
                );
              }

              return (
                <Link
                  key={item.label}
                  href={item.href}
                  className={className}
                  aria-current={item.active ? "page" : undefined}
                >
                  <Icon className="size-4" />
                  <span>{item.label}</span>
                </Link>
              );
            })}
          </nav>
        </div>

        <div className="ocean-app-header__actions">
          {viewerEmail ? (
            <span className="hidden max-w-56 truncate text-sm text-muted-foreground md:inline">
              {viewerEmail}
            </span>
          ) : null}
          <SignOutLink className="ocean-sign-out-button" aria-label="Sign out">
            <LogOutIcon className="size-4" />
            <span>Sign out</span>
          </SignOutLink>
        </div>
      </header>

      <main ref={scrollRef} className="ocean-app-scroll">
        <div className="ocean-app-content">{children}</div>
      </main>
    </div>
  );
}

export function DashboardShell({
  children,
  isAdmin,
  machines,
  saasMode,
  viewerEmail,
}: DashboardShellProps) {
  const pathname = usePathname() ?? "/dashboard";
  const searchParams = useSearchParams();
  const activeMachineId = activeMachineIdFromPath(pathname);
  const queryMachineId = searchParams.get("machine") ?? searchParams.get("machineId");
  const selectedMachineId = activeMachineId ?? queryMachineId;
  const activeMachine = useMemo(
    () => machines.find((machine) => machine.id === selectedMachineId) ?? null,
    [selectedMachineId, machines]
  );
  const showMachineFleet = isAdmin || machines.length > 1;
  const isChatSurface = /^\/dashboard\/machines\/[^/]+\/chat\/?$/u.test(pathname);

  if (isChatSurface) {
    return <div className="ocean-shell ocean-shell--chat">{children}</div>;
  }

  return (
    <div className="ocean-shell">
      <DashboardAppSection
        activeMachine={activeMachine}
        machines={machines}
        pathname={pathname}
        saasMode={saasMode}
        showMachineFleet={showMachineFleet}
        viewerEmail={viewerEmail}
      >
        {children}
      </DashboardAppSection>
    </div>
  );
}
