-- Add workspace column for multi-tenant isolation.
ALTER TABLE objects ADD COLUMN workspace TEXT NOT NULL DEFAULT '';

-- Backfill workspace-scoped object types to 'default'.
UPDATE objects SET workspace = 'default'
  WHERE workspace = '' AND name IS NOT NULL
  AND object_type IN ('sandbox', 'provider', 'service_endpoint', 'inference_route', 'ssh_session');

-- Replace global name uniqueness with workspace-scoped uniqueness.
DROP INDEX IF EXISTS objects_name_uq;
CREATE UNIQUE INDEX objects_name_uq
    ON objects (object_type, workspace, name)
    WHERE name IS NOT NULL;
