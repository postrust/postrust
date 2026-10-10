-- Row-level security as a deployment sets it up: the function logs in as a
-- role with no privileges of its own, which becomes the role a token names.
--
-- INHERIT, not the NOINHERIT PostgREST's documentation recommends, for now:
-- the schema cache reads `information_schema`, which lists only what the
-- connecting role can use, so a NOINHERIT authenticator sees no tables at all
-- -- in the server binary as much as here. That is a schema cache bug of its
-- own, and when it is fixed this goes back to NOINHERIT.
CREATE ROLE authenticator LOGIN PASSWORD 'authenticator';
CREATE ROLE web_anon NOLOGIN;
CREATE ROLE web_user NOLOGIN;
GRANT web_anon, web_user TO authenticator;

CREATE TABLE todos (
  id serial PRIMARY KEY,
  owner text NOT NULL,
  task text NOT NULL,
  public boolean NOT NULL DEFAULT false
);
INSERT INTO todos (owner, task, public) VALUES
  ('alice', 'alice public', true),
  ('alice', 'alice private', false),
  ('bob', 'bob private', false);

ALTER TABLE todos ENABLE ROW LEVEL SECURITY;
CREATE POLICY anon_read ON todos FOR SELECT TO web_anon USING (public);
CREATE POLICY own ON todos FOR ALL TO web_user
  USING (owner = current_setting('request.jwt.claims', true)::json->>'sub')
  WITH CHECK (owner = current_setting('request.jwt.claims', true)::json->>'sub');

CREATE FUNCTION whoami() RETURNS text LANGUAGE sql STABLE AS $$
  SELECT current_user || ':' || coalesce(current_setting('request.jwt.claims', true)::json->>'sub', '')
$$;

GRANT USAGE ON SCHEMA public TO web_anon, web_user;
GRANT SELECT ON todos TO web_anon;
GRANT ALL ON todos TO web_user;
GRANT USAGE ON SEQUENCE todos_id_seq TO web_user;
GRANT EXECUTE ON FUNCTION whoami() TO web_anon, web_user;
