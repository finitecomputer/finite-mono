"use client";

import type { ReactNode } from "react";
import { useEffect } from "react";
import {
  BanIcon,
  CornerDownLeftIcon,
  SearchIcon,
  TriangleAlertIcon,
} from "lucide-react";

import type { SlashCommand } from "@/lib/slash-commands";

export function slashOptionId(listboxId: string, index: number) {
  return `${listboxId}-option-${index}`;
}

// Popover above the composer. The textarea keeps focus and drives the
// highlight through aria-activedescendant; rows insert on click.
export function SlashCommandPicker({
  listboxId,
  query,
  commands,
  highlighted,
  enterInserts,
  blocked,
  onHighlight,
  onInsert,
}: {
  listboxId: string;
  query: string;
  commands: SlashCommand[];
  highlighted: number;
  enterInserts: boolean;
  blocked: SlashCommand | null;
  onHighlight: (index: number) => void;
  onInsert: (command: SlashCommand) => void;
}) {
  useEffect(() => {
    document
      .getElementById(slashOptionId(listboxId, highlighted))
      ?.scrollIntoView({ block: "nearest" });
  }, [highlighted, listboxId, commands]);

  if (blocked) {
    return (
      <div className="finite-chat__slash" aria-live="polite">
        <div className="finite-chat__slash-empty">
          <span className="finite-chat__slash-empty-icon" aria-hidden>
            <BanIcon className="size-4" />
          </span>
          <div>
            <p className="finite-chat__slash-empty-title">
              <code>/{blocked.name}</code>{" isn't available in Finite."}
            </p>
            <p>{blocked.reason}</p>
          </div>
        </div>
        <SlashFooter closable={false}>
          <span className="finite-chat__slash-count">Can&apos;t be sent from chat</span>
        </SlashFooter>
      </div>
    );
  }

  if (commands.length === 0) {
    return (
      <div className="finite-chat__slash" aria-live="polite">
        <div className="finite-chat__slash-empty">
          <span className="finite-chat__slash-empty-icon" aria-hidden>
            <SearchIcon className="size-4" />
          </span>
          <div>
            <p className="finite-chat__slash-empty-title">
              {"No listed commands match "}<code>/{query}</code>
            </p>
            <p>Enter sends it to your agent.</p>
          </div>
        </div>
        <SlashFooter sendable persistent>
          <span className="finite-chat__slash-count">0 matches</span>
        </SlashFooter>
      </div>
    );
  }

  const sections = query
    ? [{ label: `Matching "/${query}"`, start: 0, rows: commands }]
    : sectionByTier(commands);

  return (
    <div className="finite-chat__slash">
      <div
        id={listboxId}
        role="listbox"
        aria-label="Slash commands"
        className="finite-chat__slash-list"
      >
        {sections.map((section) => (
          <div key={section.label} role="group" aria-label={section.label}>
            <div className="finite-chat__slash-section" aria-hidden>
              {section.label}
            </div>
            {section.rows.map((command, offset) => {
              const index = section.start + offset;
              const active = index === highlighted;
              const warn = command.tier === "not_recommended";
              const nameMatched = command.name.startsWith(query.toLowerCase());
              return (
                <div
                  key={command.name}
                  id={slashOptionId(listboxId, index)}
                  role="option"
                  aria-selected={active}
                  className={`finite-chat__slash-row${active ? " is-active" : ""}${
                    warn ? " is-warning" : ""
                  }`}
                  onMouseDown={(event) => event.preventDefault()}
                  onMouseMove={() => {
                    if (!active) onHighlight(index);
                  }}
                  onClick={() => onInsert(command)}
                >
                  <span className="finite-chat__slash-sig">
                    <code className="finite-chat__slash-name">
                      /<MatchText text={command.name} query={nameMatched ? query : ""} />
                    </code>
                    {command.args ? (
                      <code className="finite-chat__slash-args">{command.args}</code>
                    ) : null}
                  </span>
                  <span className="finite-chat__slash-description">
                    <MatchText text={command.description} query={nameMatched ? "" : query} />
                  </span>
                  <span className="finite-chat__slash-end">
                    {warn ? (
                      <span className="finite-chat__slash-pill">Not recommended</span>
                    ) : null}
                    {active && enterInserts ? (
                      <span className="finite-chat__slash-enter" aria-hidden>
                        <CornerDownLeftIcon className="size-3" />
                      </span>
                    ) : null}
                  </span>
                  {warn && command.reason ? (
                    <span className="finite-chat__slash-reason">
                      <TriangleAlertIcon className="size-3.5" aria-hidden />
                      {command.reason}
                    </span>
                  ) : null}
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <SlashFooter navigable enterInserts={enterInserts} sendable={!enterInserts}>
        <span className="finite-chat__slash-count">
          {commands.length} {query ? (commands.length === 1 ? "match" : "matches") : "commands"}
        </span>
      </SlashFooter>
    </div>
  );
}

function SlashFooter({
  navigable = false,
  enterInserts = false,
  sendable = false,
  persistent = false,
  closable = true,
  children,
}: {
  navigable?: boolean;
  enterInserts?: boolean;
  sendable?: boolean;
  persistent?: boolean;
  closable?: boolean;
  children: ReactNode;
}) {
  return (
    <div className={`finite-chat__slash-footer${persistent ? " is-persistent" : ""}`}>
      {navigable ? (
        <>
          <span>
            <kbd>↑</kbd>
            <kbd>↓</kbd> Navigate
          </span>
          <span>
            {enterInserts ? <><kbd>Enter</kbd> or </> : null}
            <kbd>Tab</kbd> Insert
          </span>
        </>
      ) : null}
      {sendable ? (
        <span>
          <kbd>Enter</kbd> Send
        </span>
      ) : null}
      {closable ? (
        <span>
          <kbd>Esc</kbd> Close
        </span>
      ) : null}
      {children}
    </div>
  );
}

function MatchText({ text, query }: { text: string; query: string }) {
  const at = query ? text.toLowerCase().indexOf(query.toLowerCase()) : -1;
  if (at < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, at)}
      <mark>{text.slice(at, at + query.length)}</mark>
      {text.slice(at + query.length)}
    </>
  );
}

function sectionByTier(commands: SlashCommand[]) {
  const suggested = commands.filter((command) => command.tier === "suggested");
  const rest = commands.filter((command) => command.tier !== "suggested");
  return [
    { label: "Suggested", start: 0, rows: suggested },
    { label: "All commands", start: suggested.length, rows: rest },
  ].filter((section) => section.rows.length > 0);
}
