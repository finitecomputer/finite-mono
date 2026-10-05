-- Older campaigns keep their hash and remain redeemable. Their plaintext cannot
-- be recovered; an operator may explicitly replace it without changing seats.
ALTER TABLE trial_campaigns
    ADD COLUMN IF NOT EXISTS code TEXT CHECK (code IS NULL OR length(code) BETWEEN 8 AND 64),
    ADD COLUMN IF NOT EXISTS code_revision INTEGER NOT NULL DEFAULT 0 CHECK (code_revision >= 0);
