export type DesignTokenGroup = "color" | "radius" | "size" | "control" | "typography";

export type DesignTokenEntry = {
  name: string;
  cssVar: string;
  group: DesignTokenGroup;
  description: string;
};

export const SHADCN_TOKENS: DesignTokenEntry[] = [
  { name: "background", cssVar: "--background", group: "color", description: "Page surface" },
  { name: "foreground", cssVar: "--foreground", group: "color", description: "Primary text" },
  { name: "card", cssVar: "--card", group: "color", description: "Card surface" },
  { name: "primary", cssVar: "--primary", group: "color", description: "Primary actions" },
  { name: "muted", cssVar: "--muted", group: "color", description: "Muted surfaces" },
  { name: "muted-foreground", cssVar: "--muted-foreground", group: "color", description: "Secondary copy" },
  { name: "border", cssVar: "--border", group: "color", description: "Default borders" },
  { name: "destructive", cssVar: "--destructive", group: "color", description: "Destructive actions" },
  { name: "ring", cssVar: "--ring", group: "color", description: "Focus rings" },
];

export const OCEAN_TOKENS: DesignTokenEntry[] = [
  { name: "bg-window", cssVar: "--bg-window", group: "color", description: "App window background" },
  { name: "bg-sidebar", cssVar: "--bg-sidebar", group: "color", description: "Sidebar surface" },
  { name: "bg-hover", cssVar: "--bg-hover", group: "color", description: "Hover surfaces" },
  { name: "bg-active", cssVar: "--bg-active", group: "color", description: "Active rows and bubbles" },
  { name: "bg-elevated", cssVar: "--bg-elevated", group: "color", description: "Raised controls" },
  { name: "text-primary", cssVar: "--text-primary", group: "color", description: "Primary text" },
  { name: "text-secondary", cssVar: "--text-secondary", group: "color", description: "Secondary text" },
  { name: "text-tertiary", cssVar: "--text-tertiary", group: "color", description: "Timestamps and hints" },
  { name: "text-icon", cssVar: "--text-icon", group: "color", description: "Icon buttons" },
  { name: "link", cssVar: "--link", group: "color", description: "Links and focus accents" },
  { name: "surface-chip", cssVar: "--surface-chip", group: "color", description: "Subtle chips" },
  { name: "border-strong", cssVar: "--border-strong", group: "color", description: "Emphasized borders" },
  { name: "status-success-fg", cssVar: "--status-success-fg", group: "color", description: "Success text" },
  { name: "status-warning-fg", cssVar: "--status-warning-fg", group: "color", description: "Warning text" },
  { name: "status-error-fg", cssVar: "--status-error-fg", group: "color", description: "Error text" },
  { name: "radius-icon", cssVar: "--radius-icon", group: "radius", description: "Icon controls" },
  { name: "radius-pill", cssVar: "--radius-pill", group: "radius", description: "Pills and send buttons" },
  { name: "radius-bubble", cssVar: "--radius-bubble", group: "radius", description: "Message bubbles" },
  { name: "radius-composer", cssVar: "--radius-composer", group: "radius", description: "Chat composer" },
  { name: "radius-segmented", cssVar: "--radius-segmented", group: "radius", description: "Segmented controls" },
  { name: "radius-badge", cssVar: "--radius-badge", group: "radius", description: "Badges" },
  { name: "radius-button", cssVar: "--radius-button", group: "radius", description: "Buttons" },
  { name: "size-control", cssVar: "--size-control", group: "size", description: "Icon controls" },
  { name: "size-control-composer", cssVar: "--size-control-composer", group: "size", description: "Composer controls" },
];

export const CONTROL_PRIMITIVES = [
  {
    name: "Icon button",
    className: "ocean-icon-button",
    alias: "ocean-control-icon",
    use: "Topbar, sidebar collapse, compact actions",
  },
  {
    name: "Pill button",
    className: "ocean-pill-button",
    alias: "ocean-control-pill",
    use: "Outlined text actions",
  },
  {
    name: "Tool button",
    className: "ocean-control-tool",
    alias: "finite-chat__tool-button",
    use: "Composer toolbar icons",
  },
  {
    name: "Command button",
    className: "ocean-control-command",
    alias: "finite-chat__command-button",
    use: "Composer command trigger",
  },
  {
    name: "Send button",
    className: "ocean-control-send",
    alias: "finite-chat__send-button",
    use: "Primary composer submit",
  },
  {
    name: "Segmented control",
    className: "ocean-section-tabs",
    alias: "ocean-segmented-control",
    use: "Dashboard section navigation",
  },
] as const;

export type StatusBadgeState = "pending" | "in_progress" | "complete" | "blocked";

const STATUS_CLASS: Record<StatusBadgeState, string> = {
  complete: "status-badge status-badge--success",
  in_progress: "status-badge status-badge--warning",
  blocked: "status-badge status-badge--error",
  pending: "status-badge status-badge--neutral",
};

export function statusBadgeClassName(status: StatusBadgeState) {
  return STATUS_CLASS[status];
}
