-- Postrust Test Fixtures
-- Sample data for testing

-- =============================================================================
-- USERS
-- =============================================================================

INSERT INTO public.users (id, name, email, role, status, metadata) VALUES
    (1, 'Alice Johnson', 'alice@example.com', 'admin', 'active', '{"department": "Engineering", "level": 3}'),
    (2, 'Bob Smith', 'bob@example.com', 'user', 'active', '{"department": "Marketing", "level": 1}'),
    (3, 'Carol Williams', 'carol@example.com', 'user', 'active', '{"department": "Sales", "level": 2}'),
    (4, 'David Brown', 'david@example.com', 'moderator', 'active', '{"department": "Support", "level": 2}'),
    (5, 'Eve Davis', 'eve@example.com', 'user', 'inactive', '{"department": "HR", "level": 1}'),
    (6, 'Frank Miller', 'frank@example.com', 'user', 'active', '{"department": "Engineering", "level": 2}'),
    (7, 'Grace Lee', 'grace@example.com', 'admin', 'active', '{"department": "Engineering", "level": 3}'),
    (8, 'Henry Wilson', 'henry@example.com', 'user', 'pending', '{"department": "Marketing", "level": 1}'),
    (9, 'Ivy Martinez', 'ivy@example.com', 'user', 'active', '{"department": "Design", "level": 2}'),
    (10, 'Jack Taylor', 'jack@example.com', 'user', 'active', '{"department": "Engineering", "level": 1}')
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    email = EXCLUDED.email,
    role = EXCLUDED.role,
    status = EXCLUDED.status,
    metadata = EXCLUDED.metadata;

-- Reset sequence
SELECT setval('users_id_seq', 10, true);

-- =============================================================================
-- CATEGORIES
-- =============================================================================

INSERT INTO public.categories (id, name, description, parent_id) VALUES
    (1, 'Technology', 'Technology related posts', NULL),
    (2, 'Programming', 'Programming tutorials and tips', 1),
    (3, 'Web Development', 'Web development articles', 2),
    (4, 'Databases', 'Database related content', 1),
    (5, 'Lifestyle', 'Lifestyle and personal posts', NULL),
    (6, 'Travel', 'Travel stories and guides', 5),
    (7, 'Food', 'Food and recipes', 5),
    (8, 'Business', 'Business and entrepreneurship', NULL),
    (9, 'Startups', 'Startup ecosystem', 8),
    (10, 'Marketing', 'Marketing strategies', 8)
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    parent_id = EXCLUDED.parent_id;

SELECT setval('categories_id_seq', 10, true);

-- =============================================================================
-- POSTS
-- =============================================================================

INSERT INTO public.posts (id, user_id, title, content, status, tags, published_at) VALUES
    (1, 1, 'Getting Started with PostgreSQL', 'PostgreSQL is a powerful, open source object-relational database system...', 'published', ARRAY['postgresql', 'database', 'tutorial'], NOW() - INTERVAL '10 days'),
    (2, 1, 'Advanced SQL Techniques', 'Learn about window functions, CTEs, and more...', 'published', ARRAY['sql', 'advanced', 'tips'], NOW() - INTERVAL '8 days'),
    (3, 2, 'Introduction to REST APIs', 'REST APIs are the backbone of modern web applications...', 'published', ARRAY['api', 'rest', 'web'], NOW() - INTERVAL '7 days'),
    (4, 3, 'Building Scalable Systems', 'Scalability is crucial for modern applications...', 'published', ARRAY['architecture', 'scalability'], NOW() - INTERVAL '5 days'),
    (5, 1, 'Draft Post - Work in Progress', 'This is a draft post that is not yet published...', 'draft', ARRAY['draft'], NULL),
    (6, 4, 'Customer Support Best Practices', 'How to provide excellent customer support...', 'published', ARRAY['support', 'customer-service'], NOW() - INTERVAL '3 days'),
    (7, 6, 'Rust for Web Development', 'Why Rust is great for building web services...', 'published', ARRAY['rust', 'web', 'programming'], NOW() - INTERVAL '2 days'),
    (8, 7, 'DevOps Fundamentals', 'Introduction to DevOps practices and tools...', 'published', ARRAY['devops', 'ci-cd'], NOW() - INTERVAL '1 day'),
    (9, 9, 'UI/UX Design Principles', 'Essential design principles for modern applications...', 'published', ARRAY['design', 'ux', 'ui'], NOW()),
    (10, 2, 'Marketing Strategies for Tech', 'How to market your tech product effectively...', 'draft', ARRAY['marketing', 'strategy'], NULL)
ON CONFLICT (id) DO UPDATE SET
    user_id = EXCLUDED.user_id,
    title = EXCLUDED.title,
    content = EXCLUDED.content,
    status = EXCLUDED.status,
    tags = EXCLUDED.tags,
    published_at = EXCLUDED.published_at;

SELECT setval('posts_id_seq', 10, true);

-- =============================================================================
-- POST CATEGORIES
-- =============================================================================

INSERT INTO public.post_categories (post_id, category_id) VALUES
    (1, 4),  -- PostgreSQL -> Databases
    (2, 4),  -- SQL -> Databases
    (3, 3),  -- REST APIs -> Web Development
    (4, 1),  -- Scalable Systems -> Technology
    (6, 8),  -- Customer Support -> Business
    (7, 2),  -- Rust -> Programming
    (7, 3),  -- Rust -> Web Development
    (8, 1),  -- DevOps -> Technology
    (9, 1),  -- UI/UX -> Technology
    (10, 10) -- Marketing -> Marketing
ON CONFLICT DO NOTHING;

-- =============================================================================
-- COMMENTS
-- =============================================================================

INSERT INTO public.comments (id, post_id, user_id, content) VALUES
    (1, 1, 2, 'Great introduction to PostgreSQL! Very helpful.'),
    (2, 1, 3, 'Thanks for sharing these tips.'),
    (3, 2, 4, 'Window functions are incredibly powerful.'),
    (4, 3, 1, 'REST APIs are indeed fundamental to web development.'),
    (5, 3, 6, 'Nice explanation of the concepts.'),
    (6, 4, 7, 'Scalability is often overlooked in early stages.'),
    (7, 7, 1, 'Rust is amazing for performance-critical applications.'),
    (8, 7, 10, 'I''ve been using Rust for a year now, great choice!'),
    (9, 8, 9, 'DevOps has transformed how we deliver software.'),
    (10, 9, 2, 'These design principles are spot on!')
ON CONFLICT (id) DO UPDATE SET
    post_id = EXCLUDED.post_id,
    user_id = EXCLUDED.user_id,
    content = EXCLUDED.content;

SELECT setval('comments_id_seq', 10, true);

-- =============================================================================
-- PRODUCTS
-- =============================================================================

INSERT INTO public.products (id, name, description, price, stock, category, is_active, metadata) VALUES
    (1, 'PostgreSQL Handbook', 'Complete guide to PostgreSQL', 49.99, 100, 'Books', true, '{"format": "paperback", "pages": 450}'),
    (2, 'Rust Programming Course', 'Online course for Rust beginners', 199.99, 500, 'Courses', true, '{"duration": "20 hours", "level": "beginner"}'),
    (3, 'API Design Patterns', 'Best practices for API design', 39.99, 75, 'Books', true, '{"format": "ebook", "pages": 280}'),
    (4, 'Database Stickers Pack', 'Fun stickers for developers', 9.99, 1000, 'Merchandise', true, '{"count": 10}'),
    (5, 'DevOps Toolkit', 'Essential DevOps tools and scripts', 149.99, 200, 'Software', true, '{"license": "yearly"}'),
    (6, 'Vintage SQL Mug', 'Classic SQL query mug', 14.99, 50, 'Merchandise', false, '{"capacity": "350ml"}'),
    (7, 'Web Security Guide', 'Comprehensive web security handbook', 59.99, 80, 'Books', true, '{"format": "hardcover", "pages": 520}'),
    (8, 'Cloud Architecture Course', 'Master cloud architecture patterns', 299.99, 300, 'Courses', true, '{"duration": "40 hours", "level": "advanced"}'),
    (9, 'Developer T-Shirt', 'Comfortable cotton t-shirt', 24.99, 200, 'Merchandise', true, '{"sizes": ["S", "M", "L", "XL"]}'),
    (10, 'Code Review Pro', 'Automated code review tool', 19.99, 1000, 'Software', true, '{"license": "monthly"}')
ON CONFLICT (id) DO UPDATE SET
    name = EXCLUDED.name,
    description = EXCLUDED.description,
    price = EXCLUDED.price,
    stock = EXCLUDED.stock,
    category = EXCLUDED.category,
    is_active = EXCLUDED.is_active,
    metadata = EXCLUDED.metadata;

SELECT setval('products_id_seq', 10, true);

-- =============================================================================
-- ORDERS
-- =============================================================================

INSERT INTO public.orders (id, user_id, status, total, shipping_address) VALUES
    (1, 2, 'completed', 249.98, '{"street": "123 Main St", "city": "New York", "zip": "10001", "country": "USA"}'),
    (2, 3, 'pending', 49.99, '{"street": "456 Oak Ave", "city": "Los Angeles", "zip": "90001", "country": "USA"}'),
    (3, 4, 'shipped', 174.98, '{"street": "789 Pine Rd", "city": "Chicago", "zip": "60601", "country": "USA"}'),
    (4, 6, 'completed', 59.99, '{"street": "321 Elm St", "city": "Houston", "zip": "77001", "country": "USA"}'),
    (5, 9, 'cancelled', 24.99, '{"street": "654 Maple Dr", "city": "Phoenix", "zip": "85001", "country": "USA"}')
ON CONFLICT (id) DO UPDATE SET
    user_id = EXCLUDED.user_id,
    status = EXCLUDED.status,
    total = EXCLUDED.total,
    shipping_address = EXCLUDED.shipping_address;

SELECT setval('orders_id_seq', 5, true);

-- =============================================================================
-- ORDER ITEMS
-- =============================================================================

INSERT INTO public.order_items (id, order_id, product_id, quantity, unit_price) VALUES
    (1, 1, 1, 1, 49.99),   -- PostgreSQL Handbook
    (2, 1, 2, 1, 199.99),  -- Rust Course
    (3, 2, 1, 1, 49.99),   -- PostgreSQL Handbook
    (4, 3, 5, 1, 149.99),  -- DevOps Toolkit
    (5, 3, 9, 1, 24.99),   -- T-Shirt
    (6, 4, 7, 1, 59.99),   -- Web Security Guide
    (7, 5, 9, 1, 24.99)    -- T-Shirt (cancelled order)
ON CONFLICT (id) DO UPDATE SET
    order_id = EXCLUDED.order_id,
    product_id = EXCLUDED.product_id,
    quantity = EXCLUDED.quantity,
    unit_price = EXCLUDED.unit_price;

SELECT setval('order_items_id_seq', 7, true);

-- =============================================================================
-- VERIFICATION
-- =============================================================================

SELECT 'Test fixtures loaded successfully' as status;

SELECT
    (SELECT COUNT(*) FROM public.users) as users_count,
    (SELECT COUNT(*) FROM public.posts) as posts_count,
    (SELECT COUNT(*) FROM public.comments) as comments_count,
    (SELECT COUNT(*) FROM public.categories) as categories_count,
    (SELECT COUNT(*) FROM public.products) as products_count,
    (SELECT COUNT(*) FROM public.orders) as orders_count,
    (SELECT COUNT(*) FROM public.order_items) as order_items_count;
