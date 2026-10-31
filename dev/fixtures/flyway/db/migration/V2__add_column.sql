ALTER TABLE orders ADD COLUMN note text;
ALTER TABLE orders DROP COLUMN total;
CREATE TABLE audit_log (id bigserial PRIMARY KEY, payload jsonb);
