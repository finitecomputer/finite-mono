-- End exclusivity without deleting targeting evidence or changing requests.
-- Earlier Core readers ignore this table and conservatively keep the host reserved.
CREATE TABLE IF NOT EXISTS launch_host_reservation_releases (
    source_host_id TEXT PRIMARY KEY,
    reservation_code_id TEXT NOT NULL UNIQUE,
    canary_runtime_id TEXT NOT NULL,
    released_by_workos_user_id TEXT NOT NULL,
    released_at TIMESTAMPTZ NOT NULL,
    FOREIGN KEY (reservation_code_id, source_host_id)
        REFERENCES launch_code_host_targets(launch_code_id, source_host_id)
);
