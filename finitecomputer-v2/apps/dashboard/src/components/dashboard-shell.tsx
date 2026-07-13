"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import {
  BotIcon,
  ChevronRightIcon,
  LayoutDashboardIcon,
  Layers3Icon,
  LogOutIcon,
  MessageSquareIcon,
  PanelLeftIcon,
  PlusIcon,
  ShieldCheckIcon,
  type LucideIcon,
} from "lucide-react";

import { AccountMenu } from "@/components/agent-navigation";
import { AgentSidebar } from "@/components/agent-sidebar";
import { FiniteBrand } from "@/components/finite-brand";
import { HostedChatProvider } from "@/components/hosted-chat-provider";
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
  saasMode: boolean,
  isAdmin: boolean
): SectionLink[] {
  const machineHref = machine ? `/dashboard/machines/${machine.id}` : "/dashboard";
  const chatHref = machine ? `/dashboard/machines/${machine.id}/chat` : "/dashboard";
  const skillsHref = machine ? `/dashboard/skills?machine=${encodeURIComponent(machine.id)}` : "/dashboard/skills";

  if (saasMode) {
    return [];
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
    ...(isAdmin
      ? [{
          label: "Skills",
          href: skillsHref,
          icon: Layers3Icon,
          active: pathname === "/dashboard/skills",
        }]
      : []),
  ];
}

function newAgentHref(machine: MachineNavItem | null) {
  const params = new URLSearchParams({ new: "1" });
  if (machine?.id) {
    params.set("machine", machine.id);
  }
  return `/dashboard?${params.toString()}`;
}

function MachineSwitcher({
  activeMachine,
  creatingNewAgent,
  machines,
  onNavigate,
  showNewAgent,
}: {
  activeMachine: MachineNavItem | null;
  creatingNewAgent: boolean;
  machines: MachineNavItem[];
  onNavigate?: () => void;
  showNewAgent: boolean;
}) {
  const [open, setOpen] = useState(false);

  if (machines.length === 0 && showNewAgent) {
    return (
      <div className="ocean-machine-switcher">
        <Link href={newAgentHref(activeMachine)} className="ocean-machine-switcher__button">
          <PlusIcon className="size-4" aria-hidden />
          <span className="ocean-machine-switcher__label">New agent</span>
        </Link>
      </div>
    );
  }

  return (
    <div className="ocean-machine-switcher">
      <button
        type="button"
        className="ocean-machine-switcher__button"
        aria-expanded={open}
        aria-haspopup="menu"
        onClick={() => setOpen((value) => !value)}
      >
        {creatingNewAgent ? (
          <PlusIcon className="size-4" aria-hidden />
        ) : (
          <span className="ocean-machine-switcher__dot" aria-hidden />
        )}
        <span className="ocean-machine-switcher__label">
          {creatingNewAgent
            ? "New agent"
            : activeMachine?.ownerLabel ?? machines[0]?.ownerLabel ?? "Agents"}
        </span>
        <ChevronRightIcon className={cn("ocean-machine-switcher__chevron", open && "is-open")} />
      </button>

      {open ? (
        <div className="ocean-menu ocean-machine-switcher__menu" role="menu">
          {machines.map((machine) => (
            <Link
              key={machine.id}
              href={`/dashboard/machines/${machine.id}`}
              className={cn(
                "ocean-menu-item",
                !creatingNewAgent && activeMachine?.id === machine.id && "is-active"
              )}
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
          {showNewAgent ? (
            <>
              {machines.length > 0 ? <div className="ocean-menu-separator" /> : null}
              <Link
                href={newAgentHref(activeMachine)}
                className={cn("ocean-menu-item", creatingNewAgent && "is-active")}
                role="menuitem"
                onClick={() => {
                  setOpen(false);
                  onNavigate?.();
                }}
              >
                <PlusIcon className="size-4" />
                <span>New agent</span>
              </Link>
            </>
          ) : null}
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
  isAdmin,
  isNewAgentFlow,
  saasMode,
  showMachineFleet,
  viewerEmail,
}: {
  children: React.ReactNode;
  activeMachine: MachineNavItem | null;
  machines: MachineNavItem[];
  pathname: string;
  isAdmin: boolean;
  isNewAgentFlow: boolean;
  saasMode: boolean;
  showMachineFleet: boolean;
  viewerEmail?: string | null;
}) {
  const selectedMachine = activeMachine ?? machines[0] ?? null;
  const links = sectionLinks(pathname, selectedMachine, saasMode, isAdmin);
  const scrollRef = useRef<HTMLElement>(null);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [pathname]);

  return (
    <div className="ocean-app-section">
      <header className="ocean-app-header">
        <div className="ocean-app-header__brand">
          <FiniteBrand href="/dashboard" />
        </div>

        <div className="ocean-app-header__center">
          {showMachineFleet && !saasMode ? (
            <MachineSwitcher
              activeMachine={selectedMachine}
              creatingNewAgent={isNewAgentFlow}
              machines={machines}
              showNewAgent={saasMode}
            />
          ) : null}
          {links.length > 0 ? (
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
          ) : null}
        </div>

        <div className="ocean-app-header__actions">
          {isAdmin ? (
            <Link
              href="/dashboard/admin"
              className="ocean-sign-out-button"
              aria-label="Admin Ops"
            >
              <ShieldCheckIcon className="size-4" />
              <span className="hidden md:inline">Admin Ops</span>
            </Link>
          ) : null}
          {saasMode ? (
            <AccountMenu viewerEmail={viewerEmail} />
          ) : (
            <>
              {viewerEmail ? (
                <span className="hidden max-w-56 truncate text-sm text-muted-foreground md:inline">
                  {viewerEmail}
                </span>
              ) : null}
              <SignOutLink className="ocean-sign-out-button" aria-label="Sign out">
                <LogOutIcon className="size-4" />
                <span>Sign out</span>
              </SignOutLink>
            </>
          )}
        </div>
      </header>

      <main ref={scrollRef} className="ocean-app-scroll">
        <div className="ocean-app-content">{children}</div>
      </main>
    </div>
  );
}

function AgentAppSection({
  children,
  isChatSurface,
  machine,
  machines,
  showSkills,
  viewerEmail,
}: {
  children: React.ReactNode;
  isChatSurface: boolean;
  machine: MachineNavItem;
  machines: MachineNavItem[];
  showSkills: boolean;
  viewerEmail?: string | null;
}) {
  const pathname = usePathname() ?? "";
  const scrollRef = useRef<HTMLElement>(null);
  const [collapsed, setCollapsed] = useState(false);
  const [mobileOpen, setMobileOpen] = useState(false);
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: 0 });
  }, [pathname]);

  useEffect(() => {
    const open = () => setMobileOpen(true);
    window.addEventListener("finite:open-agent-sidebar", open);
    return () => window.removeEventListener("finite:open-agent-sidebar", open);
  }, []);

  return (
    <HostedChatProvider key={machine.id} machineId={machine.id}>
      <div className={`finite-agent-shell ${collapsed ? "is-sidebar-collapsed" : ""}`}>
        <AgentSidebar
          collapsed={collapsed}
          machineId={machine.id}
          machineLabel={machine.ownerLabel}
          machineSwitcher={
            <MachineSwitcher
              activeMachine={machine}
              creatingNewAgent={false}
              machines={machines}
              onNavigate={() => setMobileOpen(false)}
              showNewAgent
            />
          }
          mobileOpen={mobileOpen}
          onCollapsedChange={setCollapsed}
          onMobileOpenChange={setMobileOpen}
          showSkills={showSkills}
          viewerEmail={viewerEmail}
        />
        <main
          ref={scrollRef}
          className={`ocean-app-scroll finite-agent-shell__content ${isChatSurface ? "is-chat" : ""}`}
        >
          {!isChatSurface ? (
            <button
              type="button"
              className="ocean-icon-button finite-agent-shell__mobile-trigger"
              aria-label="Open agent navigation"
              onClick={() => setMobileOpen(true)}
            >
              <PanelLeftIcon className="size-4" />
            </button>
          ) : null}
          {isChatSurface ? children : <div className="ocean-app-content">{children}</div>}
        </main>
      </div>
    </HostedChatProvider>
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
  const isNewAgentFlow = pathname === "/dashboard" && searchParams.get("new") === "1";
  const activeMachine = useMemo(
    () => machines.find((machine) => machine.id === selectedMachineId) ?? null,
    [selectedMachineId, machines]
  );
  const showMachineFleet = saasMode || machines.length > 1;
  const isChatSurface = /^\/dashboard\/machines\/[^/]+\/chat\/?$/u.test(pathname);
  const isAgentSurface = Boolean(
    saasMode
    && activeMachine
    && (activeMachineId || (pathname === "/dashboard/skills" && queryMachineId))
  );

  if (isAgentSurface && activeMachine) {
    return (
      <div className="ocean-shell ocean-shell--agent">
        <AgentAppSection
          isChatSurface={isChatSurface}
          machine={activeMachine}
          machines={machines}
          showSkills={isAdmin}
          viewerEmail={viewerEmail}
        >
          {children}
        </AgentAppSection>
      </div>
    );
  }

  return (
    <div className="ocean-shell">
      <DashboardAppSection
        activeMachine={activeMachine}
        machines={machines}
        pathname={pathname}
        isAdmin={isAdmin}
        isNewAgentFlow={isNewAgentFlow}
        saasMode={saasMode}
        showMachineFleet={showMachineFleet}
        viewerEmail={viewerEmail}
      >
        {isChatSurface && activeMachine ? (
          <HostedChatProvider key={activeMachine.id} machineId={activeMachine.id}>
            {children}
          </HostedChatProvider>
        ) : children}
      </DashboardAppSection>
    </div>
  );
}
