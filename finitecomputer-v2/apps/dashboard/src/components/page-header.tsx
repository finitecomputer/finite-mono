import headingStyles from "@/styles/agent-page-heading.module.css";

type PageHeaderProps = {
  eyebrow?: string;
  hierarchy?: "default" | "agent";
  title: string;
  description?: string;
  actions?: React.ReactNode;
};

export function PageHeader({ eyebrow, title, description, actions, hierarchy = "default" }: PageHeaderProps) {
  return (
    <header className="flex flex-col gap-4 md:flex-row md:items-start md:justify-between">
      <div className={hierarchy === "agent" ? `${headingStyles.stack} flex-1` : "min-w-0 space-y-2"}>
        {eyebrow ? (
          <div className="type-label text-muted-foreground">
            {eyebrow}
          </div>
        ) : null}
        <h1 className={hierarchy === "agent" ? headingStyles.title : "type-title-1"}>
          {title}
        </h1>
        {description ? (
          <p className={hierarchy === "agent" ? headingStyles.subtitle : "max-w-3xl type-body-lg text-muted-foreground"}>
            {description}
          </p>
        ) : null}
      </div>
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </header>
  );
}
