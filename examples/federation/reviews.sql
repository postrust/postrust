-- The reviews database: what people said about a product. It knows products
-- only by their key.
CREATE ROLE authenticator LOGIN NOINHERIT PASSWORD 'authenticator';
CREATE ROLE web_anon NOLOGIN;
CREATE ROLE editor NOLOGIN;
GRANT web_anon, editor TO authenticator;

CREATE TABLE products (id int PRIMARY KEY);
CREATE TABLE reviews (
  id int PRIMARY KEY,
  product_id int NOT NULL REFERENCES products,
  rating int NOT NULL,
  body text NOT NULL,
  published boolean NOT NULL DEFAULT false
);
INSERT INTO products VALUES (1), (2);
INSERT INTO reviews VALUES
  (1, 1, 5, 'Best keyboard I have owned.', true),
  (2, 1, 2, 'Pending moderation.', false),
  (3, 2, 4, 'Does what it says.', true);

-- Anyone sees published reviews; an editor sees all of them.
ALTER TABLE reviews ENABLE ROW LEVEL SECURITY;
CREATE POLICY published ON reviews FOR SELECT TO web_anon USING (published);
CREATE POLICY everything ON reviews FOR SELECT TO editor USING (true);

GRANT SELECT ON products, reviews TO web_anon, editor;
