"use client";

import { useState, type ReactNode } from "react";
import { CopyIcon, Globe2Icon, LockKeyholeIcon, MessageSquareIcon, MoreHorizontalIcon, PlusIcon, SearchIcon, UploadIcon, UsersIcon, XIcon } from "lucide-react";

import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import headingStyles from "@/styles/agent-page-heading.module.css";
import styles from "@/styles/sites-browser.module.css";
import tableStyles from "@/styles/dashboard-table.module.css";

// Presentation data only. The service adapter must supply authorized records;
// these labels are never used to grant or infer access.
export type SiteListItem = {
  id: string;
  title: string;
  url: string;
  updatedLabel?: string;
  statusLabel?: string;
  published?: boolean;
  canEdit?: boolean;
  repositoryUrl?: string;
  access: "private" | "shared" | "public";
  accessLabel: string;
  thumbnailUrl?: string;
};

export function SitesBrowser({ sites, onNewSite, onEditSite, creatingSite = false, newSiteError, listingNotConnected = false, loading = false, subtitle, inventoryStatus }: {
  sites: readonly SiteListItem[] | null;
  onNewSite: () => void;
  onEditSite: (site: SiteListItem) => void;
  creatingSite?: boolean;
  newSiteError?: string | null;
  listingNotConnected?: boolean;
  loading?: boolean;
  subtitle?: string;
  inventoryStatus?: ReactNode;
}) {
  const [query, setQuery] = useState("");
  const [shareSite, setShareSite] = useState<SiteListItem | null>(null);
  const [notice, setNotice] = useState("");
  const normalizedQuery = query.trim().toLocaleLowerCase();
  const visible = sites?.filter((site) => `${site.title} ${site.url}`.toLocaleLowerCase().includes(normalizedQuery)) ?? [];

  async function copyLink(site: SiteListItem) {
    try {
      await navigator.clipboard.writeText(site.url);
      setNotice(`Link copied for ${site.title}.`);
    } catch {
      setShareSite(site);
      setNotice("Couldn’t copy the link. Select and copy it below.");
    }
  }

  return (
    <section className={`${styles.page} ${headingStyles.page}`} aria-labelledby="sites-title">
      <header className={`${styles.heading} ${headingStyles.stack}`}>
        <h1 id="sites-title" className={headingStyles.title}>Sites</h1>
        <p className={headingStyles.subtitle}>{subtitle ?? "Turn your ideas into live websites"}</p>
      </header>

      {inventoryStatus}

      {Boolean(sites?.length) && <div className={styles.search} data-mobile-only={(sites?.length ?? 0) <= 10 && !query}>
        <SearchIcon aria-hidden="true" size={21} />
        <input aria-label="Search sites" placeholder="Search sites" value={query} onChange={(event) => setQuery(event.target.value)} type="search" />
        <button type="button" aria-label="Clear search" style={{ visibility: query ? "visible" : "hidden" }} onClick={() => setQuery("")}><XIcon size={18} /></button>
      </div>}

      <div className={`${styles.browser} ${tableStyles.surface}`}>
              <div className={styles.results}>
              {Boolean(sites?.length) && <div className={styles.columns} aria-hidden="true"><span>Site</span><span>Sharing</span><span>Actions</span></div>}
              {visible.length ? (
                <ul className={styles.list} aria-label="Sites">
                  {visible.map((site) => {
                    const AccessIcon = site.access === "private" ? LockKeyholeIcon : site.access === "public" ? Globe2Icon : UsersIcon;
                    return (
                      <li key={site.id} className={styles.row}>
                        <a href={site.published === false ? undefined : site.url} aria-disabled={site.published === false || undefined} target="_blank" rel="noreferrer" className={styles.site}>
                          <span className={styles.thumbnail}>
                            {/* Native image keeps thumbnails independent of remote optimizer configuration. */}
                            {/* eslint-disable-next-line @next/next/no-img-element */}
                            {site.thumbnailUrl ? <img src={site.thumbnailUrl} alt="" loading="lazy" onError={(event) => { event.currentTarget.style.display = "none"; }} /> : null}
                            <Globe2Icon aria-hidden="true" size={28} />
                          </span>
                          <span className={styles.details}><strong>{site.title}</strong><span>{(site.statusLabel || site.updatedLabel) && <>{site.statusLabel || site.updatedLabel}<span aria-hidden="true"> · </span></>}{site.url.replace(/^https?:\/\//u, "").replace(/\/$/u, "")}</span></span>
                        </a>
                        <span className={styles.access}><AccessIcon size={16} strokeWidth={1.5} aria-hidden="true" />{site.accessLabel}</span>
                        <div className={styles.actions}>
                          <div className={styles.desktopActions}>
                            <Tooltip>
                              <TooltipTrigger asChild><Button type="button" variant="outline" size="icon-sm" aria-label={`Copy link to ${site.title}`} disabled={site.published === false} onClick={() => void copyLink(site)}><CopyIcon className="size-3.5" aria-hidden="true" /></Button></TooltipTrigger>
                              <TooltipContent sideOffset={6}>Copy link</TooltipContent>
                            </Tooltip>
                            <Tooltip>
                              <TooltipTrigger asChild><Button type="button" variant="outline" size="icon-sm" aria-label={`Share ${site.title}`} disabled={site.published === false} onClick={() => { setNotice(""); setShareSite(site); }}><UploadIcon className="size-3.5" aria-hidden="true" /></Button></TooltipTrigger>
                              <TooltipContent sideOffset={6}>Share site</TooltipContent>
                            </Tooltip>
                            <Tooltip>
                              <TooltipTrigger asChild><Button type="button" variant="outline" size="sm" aria-label={`Edit ${site.title} in chat`} disabled={creatingSite || site.canEdit === false} onClick={() => onEditSite(site)}><MessageSquareIcon aria-hidden="true" />Edit</Button></TooltipTrigger>
                              <TooltipContent sideOffset={6}>Edit in chat</TooltipContent>
                            </Tooltip>
                          </div>
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild><button type="button" className={styles.more} aria-label={`More options for ${site.title}`}><MoreHorizontalIcon size={21} /></button></DropdownMenuTrigger>
                            <DropdownMenuContent align="end">
                              <DropdownMenuItem disabled={site.published === false} onSelect={() => void copyLink(site)}><CopyIcon />Copy link</DropdownMenuItem>
                              <DropdownMenuItem disabled={site.published === false} onSelect={() => { setNotice(""); setShareSite(site); }}><UploadIcon />Share</DropdownMenuItem>
                              <DropdownMenuItem disabled={creatingSite || site.canEdit === false} onSelect={() => onEditSite(site)}><MessageSquareIcon />Edit</DropdownMenuItem>
                            </DropdownMenuContent>
                          </DropdownMenu>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              ) : (
                <div className={styles.empty}>
                  {normalizedQuery && sites ? <SearchIcon aria-hidden="true" /> : <Globe2Icon aria-hidden="true" />}
                  <h2>{loading ? "Loading sites…" : sites === null ? (listingNotConnected ? "Site listings aren’t available yet" : "Sites are unavailable right now") : normalizedQuery ? "No sites found" : "Your sites will show up here"}</h2>
                  <p>{loading ? "Reading this agent’s sites." : sites === null ? (listingNotConnected ? "You can still create and work on sites with your agent in chat." : "We couldn’t load your sites. Please try again later.") : normalizedQuery ? "Try another site name or web address." : "Start with an idea, push the limits, and make something awesome. Create your next site in chat."}</p>
                  {normalizedQuery && sites && <button type="button" className={styles.share} onClick={() => setQuery("")}>Clear search</button>}
                  {!loading && (!normalizedQuery || !sites?.length) && <button type="button" className={styles.newSite} disabled={creatingSite} onClick={onNewSite}>{creatingSite ? "Opening chat…" : "Create a site"}</button>}
                </div>
              )}
              </div>
      </div>
      <footer className={styles.footer}>
        {Boolean(sites?.length) && <button type="button" className={styles.newSite} disabled={creatingSite} onClick={onNewSite}><PlusIcon size={18} aria-hidden="true" />{creatingSite ? "Opening chat…" : "New site"}</button>}
        <p role="status" className={styles.notice}>{newSiteError || (shareSite ? "" : notice)}</p>
      </footer>

      <Dialog open={shareSite !== null} onOpenChange={(open) => { if (!open) { setShareSite(null); setNotice(""); } }}>
        <DialogContent>
          <DialogHeader><DialogTitle>Share site</DialogTitle><DialogDescription>{shareSite?.title}</DialogDescription></DialogHeader>
          <p className="text-sm text-muted-foreground">Copy a link to this site. This doesn’t change who has access.</p>
          <input className={styles.linkInput} aria-label="Site link" readOnly value={shareSite?.url ?? ""} onFocus={(event) => event.currentTarget.select()} />
          <button type="button" className={styles.share} onClick={() => { if (shareSite) void copyLink(shareSite); }}><CopyIcon size={17} />Copy link</button>
          <p role="status" className="text-sm text-muted-foreground">{notice}</p>
        </DialogContent>
      </Dialog>
    </section>
  );
}
