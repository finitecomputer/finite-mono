-- Cohort admission preserves the root host reservation and existing readers.
ALTER TABLE launch_code_host_targets
    ADD COLUMN IF NOT EXISTS cohort_of_launch_code_id TEXT;
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conrelid='launch_code_host_targets'::regclass
                   AND conname='launch_code_host_targets_cohort_parent') THEN
        ALTER TABLE launch_code_host_targets ADD CONSTRAINT launch_code_host_targets_cohort_parent
            FOREIGN KEY (cohort_of_launch_code_id, source_host_id)
            REFERENCES launch_code_host_targets(launch_code_id, source_host_id);
        ALTER TABLE launch_code_host_targets ADD CONSTRAINT launch_code_host_targets_cohort_kind
            CHECK (cohort_of_launch_code_id IS NULL OR
                   (cohort_of_launch_code_id <> launch_code_id AND retry_of_launch_code_id IS NULL));
    END IF;
END $$;
-- Both earlier migrations retain this name on replay, including N-1 startup.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_index
               WHERE indexrelid=to_regclass('launch_code_host_targets_host_idx')
                 AND position('cohort_of_launch_code_id' IN pg_get_expr(indpred, indrelid))=0) THEN
        DROP INDEX launch_code_host_targets_host_idx;
    END IF;
END $$;
CREATE UNIQUE INDEX IF NOT EXISTS launch_code_host_targets_host_idx
    ON launch_code_host_targets(source_host_id)
    WHERE retry_of_launch_code_id IS NULL AND cohort_of_launch_code_id IS NULL;
