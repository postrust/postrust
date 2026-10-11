-- The catalog database: what a product is.
--
-- Postrust logs in as `authenticator`, which has no privileges of its own and
-- becomes the role a request's token names (`web_anon` without one).
CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD 'authenticator';
CREATE ROLE web_anon NOLOGIN;
CREATE ROLE editor NOLOGIN;
GRANT web_anon, editor TO authenticator;

CREATE TABLE products (
  id int PRIMARY KEY,
  name text NOT NULL,
  price_cents int NOT NULL
);
INSERT INTO products VALUES
  (1, 'Mechanical keyboard', 12900),
  (2, 'USB-C dock', 8900);

GRANT SELECT ON products TO web_anon, editor;
