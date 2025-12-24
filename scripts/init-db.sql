-- Postrust Test Database Initialization
-- This script sets up the database schema for testing

-- Create roles
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'web_anon') THEN
        CREATE ROLE web_anon NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'web_user') THEN
        CREATE ROLE web_user NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'web_admin') THEN
        CREATE ROLE web_admin NOLOGIN;
    END IF;
END
$$;

-- Grant connect permissions
GRANT USAGE ON SCHEMA public TO web_anon, web_user, web_admin;

-- Create API schema
CREATE SCHEMA IF NOT EXISTS api;
GRANT USAGE ON SCHEMA api TO web_anon, web_user, web_admin;

-- =============================================================================
-- PUBLIC SCHEMA TABLES
-- =============================================================================

-- Users table
CREATE TABLE IF NOT EXISTS public.users (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) UNIQUE NOT NULL,
    role VARCHAR(50) DEFAULT 'user',
    status VARCHAR(20) DEFAULT 'active',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Create index for email lookups
CREATE INDEX IF NOT EXISTS idx_users_email ON public.users(email);
CREATE INDEX IF NOT EXISTS idx_users_status ON public.users(status);

-- Posts table
CREATE TABLE IF NOT EXISTS public.posts (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES public.users(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    content TEXT,
    status VARCHAR(20) DEFAULT 'draft',
    tags TEXT[] DEFAULT '{}',
    published_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_posts_user_id ON public.posts(user_id);
CREATE INDEX IF NOT EXISTS idx_posts_status ON public.posts(status);

-- Comments table
CREATE TABLE IF NOT EXISTS public.comments (
    id SERIAL PRIMARY KEY,
    post_id INTEGER REFERENCES public.posts(id) ON DELETE CASCADE,
    user_id INTEGER REFERENCES public.users(id) ON DELETE SET NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_comments_post_id ON public.comments(post_id);

-- Categories table
CREATE TABLE IF NOT EXISTS public.categories (
    id SERIAL PRIMARY KEY,
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    parent_id INTEGER REFERENCES public.categories(id),
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Post-Category junction table (many-to-many)
CREATE TABLE IF NOT EXISTS public.post_categories (
    post_id INTEGER REFERENCES public.posts(id) ON DELETE CASCADE,
    category_id INTEGER REFERENCES public.categories(id) ON DELETE CASCADE,
    PRIMARY KEY (post_id, category_id)
);

-- Products table (for additional testing)
CREATE TABLE IF NOT EXISTS public.products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    price DECIMAL(10, 2) NOT NULL,
    stock INTEGER DEFAULT 0,
    category VARCHAR(100),
    is_active BOOLEAN DEFAULT true,
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Orders table
CREATE TABLE IF NOT EXISTS public.orders (
    id SERIAL PRIMARY KEY,
    user_id INTEGER REFERENCES public.users(id),
    status VARCHAR(50) DEFAULT 'pending',
    total DECIMAL(10, 2) DEFAULT 0,
    shipping_address JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Order items table
CREATE TABLE IF NOT EXISTS public.order_items (
    id SERIAL PRIMARY KEY,
    order_id INTEGER REFERENCES public.orders(id) ON DELETE CASCADE,
    product_id INTEGER REFERENCES public.products(id),
    quantity INTEGER NOT NULL DEFAULT 1,
    unit_price DECIMAL(10, 2) NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- =============================================================================
-- VIEWS
-- =============================================================================

-- Active users view
CREATE OR REPLACE VIEW public.active_users AS
SELECT id, name, email, role, created_at
FROM public.users
WHERE status = 'active';

-- Published posts view
CREATE OR REPLACE VIEW public.published_posts AS
SELECT p.*, u.name as author_name, u.email as author_email
FROM public.posts p
LEFT JOIN public.users u ON p.user_id = u.id
WHERE p.status = 'published';

-- =============================================================================
-- STORED PROCEDURES / FUNCTIONS
-- =============================================================================

-- Get user by ID
CREATE OR REPLACE FUNCTION public.get_user(user_id INTEGER)
RETURNS SETOF public.users
LANGUAGE sql
STABLE
AS $$
    SELECT * FROM public.users WHERE id = user_id;
$$;

-- Search users
CREATE OR REPLACE FUNCTION public.search_users(query TEXT, max_results INTEGER DEFAULT 10)
RETURNS SETOF public.users
LANGUAGE sql
STABLE
AS $$
    SELECT * FROM public.users
    WHERE name ILIKE '%' || query || '%'
       OR email ILIKE '%' || query || '%'
    LIMIT max_results;
$$;

-- Get user statistics
CREATE OR REPLACE FUNCTION public.get_user_stats()
RETURNS TABLE(total_users BIGINT, active_users BIGINT, admin_users BIGINT)
LANGUAGE sql
STABLE
AS $$
    SELECT
        COUNT(*) as total_users,
        COUNT(*) FILTER (WHERE status = 'active') as active_users,
        COUNT(*) FILTER (WHERE role = 'admin') as admin_users
    FROM public.users;
$$;

-- Create user (volatile function)
CREATE OR REPLACE FUNCTION public.create_user(
    user_name VARCHAR(255),
    user_email VARCHAR(255),
    user_role VARCHAR(50) DEFAULT 'user'
)
RETURNS public.users
LANGUAGE sql
VOLATILE
AS $$
    INSERT INTO public.users (name, email, role)
    VALUES (user_name, user_email, user_role)
    RETURNING *;
$$;

-- Get posts by user
CREATE OR REPLACE FUNCTION public.get_user_posts(p_user_id INTEGER)
RETURNS SETOF public.posts
LANGUAGE sql
STABLE
AS $$
    SELECT * FROM public.posts WHERE user_id = p_user_id ORDER BY created_at DESC;
$$;

-- Calculate order total
CREATE OR REPLACE FUNCTION public.calculate_order_total(p_order_id INTEGER)
RETURNS DECIMAL(10, 2)
LANGUAGE sql
STABLE
AS $$
    SELECT COALESCE(SUM(quantity * unit_price), 0)
    FROM public.order_items
    WHERE order_id = p_order_id;
$$;

-- =============================================================================
-- ROW LEVEL SECURITY
-- =============================================================================

-- Enable RLS on users table
ALTER TABLE public.users ENABLE ROW LEVEL SECURITY;

-- Anonymous users can only see active users
CREATE POLICY users_anon_select ON public.users
    FOR SELECT
    TO web_anon
    USING (status = 'active');

-- Authenticated users can see all users
CREATE POLICY users_auth_select ON public.users
    FOR SELECT
    TO web_user
    USING (true);

-- Users can update their own record
CREATE POLICY users_auth_update ON public.users
    FOR UPDATE
    TO web_user
    USING (id = current_setting('request.jwt.claims.sub', true)::integer);

-- Admins can do everything
CREATE POLICY users_admin_all ON public.users
    FOR ALL
    TO web_admin
    USING (true);

-- Enable RLS on posts table
ALTER TABLE public.posts ENABLE ROW LEVEL SECURITY;

-- Anyone can read published posts
CREATE POLICY posts_read_published ON public.posts
    FOR SELECT
    TO web_anon, web_user
    USING (status = 'published');

-- Authors can see their own drafts
CREATE POLICY posts_author_read ON public.posts
    FOR SELECT
    TO web_user
    USING (user_id = current_setting('request.jwt.claims.sub', true)::integer);

-- Authors can update their own posts
CREATE POLICY posts_author_update ON public.posts
    FOR UPDATE
    TO web_user
    USING (user_id = current_setting('request.jwt.claims.sub', true)::integer);

-- Admins can do everything
CREATE POLICY posts_admin_all ON public.posts
    FOR ALL
    TO web_admin
    USING (true);

-- =============================================================================
-- GRANTS
-- =============================================================================

-- Public tables
GRANT SELECT ON public.users TO web_anon;
GRANT SELECT, INSERT, UPDATE ON public.users TO web_user;
GRANT ALL ON public.users TO web_admin;

GRANT SELECT ON public.posts TO web_anon;
GRANT SELECT, INSERT, UPDATE, DELETE ON public.posts TO web_user;
GRANT ALL ON public.posts TO web_admin;

GRANT SELECT ON public.comments TO web_anon, web_user;
GRANT INSERT, UPDATE, DELETE ON public.comments TO web_user;
GRANT ALL ON public.comments TO web_admin;

GRANT SELECT ON public.categories TO web_anon, web_user;
GRANT ALL ON public.categories TO web_admin;

GRANT SELECT ON public.post_categories TO web_anon, web_user;
GRANT ALL ON public.post_categories TO web_admin;

GRANT SELECT ON public.products TO web_anon, web_user;
GRANT ALL ON public.products TO web_admin;

GRANT SELECT ON public.orders TO web_user;
GRANT ALL ON public.orders TO web_admin;

GRANT SELECT ON public.order_items TO web_user;
GRANT ALL ON public.order_items TO web_admin;

-- Views
GRANT SELECT ON public.active_users TO web_anon, web_user, web_admin;
GRANT SELECT ON public.published_posts TO web_anon, web_user, web_admin;

-- Functions
GRANT EXECUTE ON FUNCTION public.get_user(INTEGER) TO web_anon, web_user, web_admin;
GRANT EXECUTE ON FUNCTION public.search_users(TEXT, INTEGER) TO web_anon, web_user, web_admin;
GRANT EXECUTE ON FUNCTION public.get_user_stats() TO web_anon, web_user, web_admin;
GRANT EXECUTE ON FUNCTION public.create_user(VARCHAR, VARCHAR, VARCHAR) TO web_user, web_admin;
GRANT EXECUTE ON FUNCTION public.get_user_posts(INTEGER) TO web_anon, web_user, web_admin;
GRANT EXECUTE ON FUNCTION public.calculate_order_total(INTEGER) TO web_user, web_admin;

-- Sequences
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO web_user, web_admin;

-- =============================================================================
-- API SCHEMA (Alternative namespace)
-- =============================================================================

-- Create API view of users (for testing schema switching)
CREATE OR REPLACE VIEW api.users AS
SELECT id, name, email, role, status, created_at
FROM public.users;

GRANT SELECT ON api.users TO web_anon, web_user;
GRANT ALL ON api.users TO web_admin;

-- API function
CREATE OR REPLACE FUNCTION api.echo(message TEXT)
RETURNS TEXT
LANGUAGE sql
STABLE
AS $$
    SELECT message;
$$;

GRANT EXECUTE ON FUNCTION api.echo(TEXT) TO web_anon, web_user, web_admin;

-- Done
SELECT 'Database initialized successfully' as status;
