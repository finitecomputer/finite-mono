-- A missing user records Core-authorized recovery without attributing it to
-- an owner who did not request it. Existing owner/operator rows are unchanged.
ALTER TABLE runtime_control_requests ALTER COLUMN requested_by_user_id DROP NOT NULL;
ALTER TABLE runtime_control_requests DROP CONSTRAINT IF EXISTS runtime_control_automatic_restart_check;
ALTER TABLE runtime_control_requests ADD CONSTRAINT runtime_control_automatic_restart_check
  CHECK (requested_by_user_id IS NOT NULL OR kind = 'restart');
