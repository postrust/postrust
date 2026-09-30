-- Database fixture for Apollo's Federation subgraph compatibility suite.
-- The data matches COMPATIBILITY.md at upstream commit
-- 346d4882d4e72a9e505369557c44a349cbca71bc.

CREATE TABLE public.product_variations (
    id text PRIMARY KEY
);

CREATE TABLE public.product_dimensions (
    id text PRIMARY KEY,
    size text,
    weight double precision,
    unit text
);

CREATE TABLE public.users (
    email text PRIMARY KEY,
    name text,
    total_products_created integer,
    years_of_employment integer NOT NULL
);

CREATE TABLE public.case_studies (
    case_number text PRIMARY KEY,
    description text
);

CREATE TABLE public.product_research (
    id text PRIMARY KEY,
    study_case_number text NOT NULL UNIQUE
        REFERENCES public.case_studies(case_number),
    outcome text
);

CREATE TABLE public.products (
    id text PRIMARY KEY,
    sku text,
    package text,
    variation_id text REFERENCES public.product_variations(id),
    dimension_id text REFERENCES public.product_dimensions(id),
    created_by_email text REFERENCES public.users(email),
    notes text,
    UNIQUE (sku, package)
);

CREATE TABLE public.product_research_links (
    product_id text NOT NULL REFERENCES public.products(id),
    research_id text NOT NULL REFERENCES public.product_research(id),
    PRIMARY KEY (product_id, research_id)
);

CREATE TABLE public.deprecated_products (
    sku text NOT NULL,
    package text NOT NULL,
    reason text,
    created_by_email text REFERENCES public.users(email),
    PRIMARY KEY (sku, package)
);

CREATE TABLE public.inventories (
    id text PRIMARY KEY
);

CREATE TABLE public.inventory_deprecated_products (
    inventory_id text NOT NULL REFERENCES public.inventories(id),
    deprecated_sku text NOT NULL,
    deprecated_package text NOT NULL,
    PRIMARY KEY (inventory_id, deprecated_sku, deprecated_package),
    FOREIGN KEY (deprecated_sku, deprecated_package)
        REFERENCES public.deprecated_products(sku, package)
);

CREATE FUNCTION public.user_average_products_created_per_year(user_row public.users)
RETURNS integer
LANGUAGE sql
STABLE
AS $$
    SELECT CASE
        WHEN user_row.total_products_created IS NULL THEN NULL
        ELSE round(
            user_row.total_products_created::numeric /
            user_row.years_of_employment
        )::integer
    END
$$;

INSERT INTO public.product_variations (id) VALUES
    ('OSS'),
    ('platform');

INSERT INTO public.product_dimensions (id, size, weight, unit) VALUES
    ('default', 'small', 1, 'kg');

INSERT INTO public.users (
    email,
    name,
    total_products_created,
    years_of_employment
) VALUES (
    'support@apollographql.com',
    'Jane Smith',
    1337,
    10
);

INSERT INTO public.case_studies (case_number, description) VALUES
    ('1234', 'Federation Study'),
    ('1235', 'Studio Study');

INSERT INTO public.product_research (id, study_case_number, outcome) VALUES
    ('federation-research', '1234', NULL),
    ('studio-research', '1235', NULL);

INSERT INTO public.products (
    id,
    sku,
    package,
    variation_id,
    dimension_id,
    created_by_email,
    notes
) VALUES
    (
        'apollo-federation',
        'federation',
        '@apollo/federation',
        'OSS',
        'default',
        'support@apollographql.com',
        NULL
    ),
    (
        'apollo-studio',
        'studio',
        '',
        'platform',
        'default',
        'support@apollographql.com',
        NULL
    );

INSERT INTO public.product_research_links (product_id, research_id) VALUES
    ('apollo-federation', 'federation-research'),
    ('apollo-studio', 'studio-research');

INSERT INTO public.deprecated_products (sku, package, reason, created_by_email)
VALUES (
    'apollo-federation-v1',
    '@apollo/federation-v1',
    'Migrate to Federation V2',
    'support@apollographql.com'
);

INSERT INTO public.inventories (id) VALUES ('apollo-oss');

INSERT INTO public.inventory_deprecated_products (
    inventory_id,
    deprecated_sku,
    deprecated_package
) VALUES (
    'apollo-oss',
    'apollo-federation-v1',
    '@apollo/federation-v1'
);
