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

-- Serialize even old targeting writers against release. Separate statements
-- matter: re-read the receipt after a root-lock wait, not in its old snapshot.
CREATE OR REPLACE FUNCTION refuse_released_launch_host_target()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM 1 FROM launch_code_host_targets
        WHERE source_host_id = NEW.source_host_id
          AND retry_of_launch_code_id IS NULL AND cohort_of_launch_code_id IS NULL
        FOR UPDATE;
    IF EXISTS (SELECT 1 FROM launch_host_reservation_releases
               WHERE source_host_id = NEW.source_host_id) THEN
        RAISE EXCEPTION 'launch host reservation has been released' USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'UPDATE' THEN
        IF EXISTS (SELECT 1 FROM launch_host_reservation_releases
                   WHERE source_host_id = OLD.source_host_id) THEN
            RAISE EXCEPTION 'released launch host targeting history is immutable' USING ERRCODE = '23514';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE OR REPLACE TRIGGER launch_host_release_fence
    BEFORE INSERT OR UPDATE ON launch_code_host_targets
    FOR EACH ROW EXECUTE FUNCTION refuse_released_launch_host_target();
