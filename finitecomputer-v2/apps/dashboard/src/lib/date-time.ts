const UTC_DATE_TIME_FORMATTER = new Intl.DateTimeFormat("en-US", {
  month: "short",
  day: "numeric",
  year: "numeric",
  hour: "numeric",
  minute: "2-digit",
  timeZone: "UTC",
  timeZoneName: "short",
});

export function formatUtcDateTime(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.valueOf()) ? value : UTC_DATE_TIME_FORMATTER.format(parsed);
}

export function formatLocalMessageTime(timestampUnixSeconds: number) {
  const date = new Date(timestampUnixSeconds * 1000);
  if (timestampUnixSeconds <= 0 || Number.isNaN(date.valueOf())) return "";
  return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}
