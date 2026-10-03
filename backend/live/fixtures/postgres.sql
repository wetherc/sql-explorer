-- The objects of the PostgreSQL live tests. Each test loads this file into
-- a new database of its own, through the driver of the application.

CREATE EXTENSION file_fdw;
CREATE SCHEMA app;

CREATE TYPE app.mood AS ENUM ('happy', 'sad');
CREATE DOMAIN app.positive AS integer CHECK (VALUE > 0);

CREATE TABLE app.orders (
    id integer PRIMARY KEY,
    total numeric,
    mood app.mood,
    qty app.positive,
    tags text[],
    note text
);
CREATE TABLE app.lines (
    id integer PRIMARY KEY,
    order_id integer REFERENCES app.orders (id),
    qty integer
);
INSERT INTO app.orders VALUES
    (1, 150, 'happy', 2, '{a,b}', 'first'),
    (2, 50, 'sad', 1, '{}', NULL);

CREATE VIEW app.big_orders AS SELECT id, total FROM app.orders WHERE total > 100;
CREATE MATERIALIZED VIEW app.order_totals AS
    SELECT mood, sum(total) AS total FROM app.orders GROUP BY mood;
CREATE MATERIALIZED VIEW app.pending_totals AS
    SELECT mood, count(*) AS orders FROM app.orders GROUP BY mood
    WITH NO DATA;

CREATE TABLE app.events (id integer, at date) PARTITION BY RANGE (at);
CREATE TABLE app.events_2025 PARTITION OF app.events
    FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
CREATE TABLE app.events_2026 PARTITION OF app.events
    FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');

CREATE SERVER files FOREIGN DATA WRAPPER file_fdw;
CREATE FOREIGN TABLE app.host_file (line text)
    SERVER files OPTIONS (filename '/etc/hostname');

CREATE FUNCTION app.pass() RETURNS trigger LANGUAGE plpgsql
    AS $$BEGIN RETURN COALESCE(NEW, OLD); END$$;
CREATE FUNCTION app.nothing() RETURNS trigger LANGUAGE plpgsql
    AS $$BEGIN RETURN NULL; END$$;

CREATE TRIGGER a_before_write BEFORE INSERT OR UPDATE OF total ON app.orders
    FOR EACH ROW EXECUTE FUNCTION app.pass();
CREATE TRIGGER b_after_delete AFTER DELETE ON app.orders
    FOR EACH ROW EXECUTE FUNCTION app.pass();
CREATE TRIGGER c_truncate AFTER TRUNCATE ON app.orders
    FOR EACH STATEMENT EXECUTE FUNCTION app.nothing();
CREATE TRIGGER d_disabled AFTER INSERT OR UPDATE OR DELETE ON app.orders
    FOR EACH ROW EXECUTE FUNCTION app.pass();
ALTER TABLE app.orders DISABLE TRIGGER d_disabled;
CREATE TRIGGER v_instead INSTEAD OF INSERT OR UPDATE OR DELETE ON app.big_orders
    FOR EACH ROW EXECUTE FUNCTION app.pass();
CREATE TRIGGER p_after_insert AFTER INSERT ON app.events
    FOR EACH ROW EXECUTE FUNCTION app.pass();
CREATE TRIGGER f_before_insert BEFORE INSERT ON app.host_file
    FOR EACH ROW EXECUTE FUNCTION app.pass();
