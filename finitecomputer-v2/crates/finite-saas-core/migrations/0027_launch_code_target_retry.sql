-- Keep the original reservation and its evidence. Permit one explicitly
-- authorized retry binding on that same host, without changing reader queries.
ALTER TABLE launch_code_host_targets
    ADD COLUMN IF NOT EXISTS retry_of_launch_code_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS launch_code_host_targets_code_host_idx
    ON launch_code_host_targets(launch_code_id, source_host_id);
CREATE UNIQUE INDEX IF NOT EXISTS launch_code_host_targets_one_retry_idx
    ON launch_code_host_targets(retry_of_launch_code_id)
    WHERE retry_of_launch_code_id IS NOT NULL;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid='launch_code_host_targets'::regclass
                   AND conname='launch_code_host_targets_retry_parent') THEN
        ALTER TABLE launch_code_host_targets ADD CONSTRAINT launch_code_host_targets_retry_parent
            FOREIGN KEY (retry_of_launch_code_id, source_host_id)
            REFERENCES launch_code_host_targets(launch_code_id, source_host_id);
        ALTER TABLE launch_code_host_targets ADD CONSTRAINT launch_code_host_targets_not_self_retry
            CHECK (retry_of_launch_code_id IS NULL OR retry_of_launch_code_id <> launch_code_id);
    END IF;
    IF EXISTS (SELECT 1 FROM pg_index WHERE indexrelid=to_regclass('launch_code_host_targets_host_idx')
               AND indpred IS NULL) THEN
        DROP INDEX launch_code_host_targets_host_idx;
    END IF;
END $$;
-- Retain the old index name: N-1 startup's CREATE INDEX IF NOT EXISTS must
-- preserve this index rather than trying to rebuild global host uniqueness.
CREATE UNIQUE INDEX IF NOT EXISTS launch_code_host_targets_host_idx
    ON launch_code_host_targets(source_host_id) WHERE retry_of_launch_code_id IS NULL;
