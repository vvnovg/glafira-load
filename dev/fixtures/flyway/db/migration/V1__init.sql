-- Заказы
CREATE TABLE orders (
    id uuid PRIMARY KEY,
    status text NOT NULL,
    total numeric(12, 2),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX orders_status_idx ON orders (status);

CREATE OR REPLACE FUNCTION touch() RETURNS trigger AS $$
BEGIN
  NEW.updated_at := now(); RETURN NEW;
END;
$$ LANGUAGE plpgsql;
