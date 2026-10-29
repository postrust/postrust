-- The table the example's /hooks/todos route reads, under a row-level
-- security policy. The example creates nothing itself; run this once against
-- the database the server connects to, as a role that may create in `api`:
--
--   psql "$PGRST_DB_URI" -f examples/custom-routes/todos.sql
--
-- It assumes the roles of scripts/init-db.sql: `web_user` for a signed-in
-- caller and `web_anon` for one who is not, with `PGRST_DB_ANON_ROLE=web_anon`
-- and `PGRST_DB_SCHEMAS=api`. A token signed with `PGRST_JWT_SECRET` and
-- carrying `{"role": "web_user", "sub": "alice"}` sees Alice's rows and no
-- one else's -- through the route as through `GET /api/todos`.

CREATE TABLE IF NOT EXISTS api.todos (
    id     serial PRIMARY KEY,
    owner  text NOT NULL DEFAULT current_setting('request.jwt.claims', true)::json->>'sub',
    title  text NOT NULL,
    done   boolean NOT NULL DEFAULT false
);

ALTER TABLE api.todos ENABLE ROW LEVEL SECURITY;

-- A caller sees, and writes, only the rows whose owner is the `sub` of their
-- token: the claims Postrust sets for REST, and for a route's `begin_as`.
DROP POLICY IF EXISTS own_todos ON api.todos;
CREATE POLICY own_todos ON api.todos TO web_user
    USING (owner = current_setting('request.jwt.claims', true)::json->>'sub')
    WITH CHECK (owner = current_setting('request.jwt.claims', true)::json->>'sub');

GRANT SELECT, INSERT, UPDATE ON api.todos TO web_user;
GRANT USAGE ON SEQUENCE api.todos_id_seq TO web_user;
-- Nothing for web_anon: an anonymous request is refused, by REST and by the
-- route alike.
