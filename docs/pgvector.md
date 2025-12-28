# pgvector Integration

Postrust provides native support for [pgvector](https://github.com/pgvector/pgvector), enabling vector similarity search directly through your REST and GraphQL APIs. Build AI-powered applications with semantic search, recommendations, and RAG (Retrieval Augmented Generation) pipelines.

## Prerequisites

1. Install pgvector extension in your PostgreSQL database:

```sql
CREATE EXTENSION IF NOT EXISTS vector;
```

2. Create tables with vector columns:

```sql
CREATE TABLE documents (
  id SERIAL PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT,
  embedding vector(1536)  -- OpenAI ada-002 dimensions
);

-- Create an index for fast similarity search
CREATE INDEX ON documents USING ivfflat (embedding vector_cosine_ops)
  WITH (lists = 100);
```

## REST API

### Querying Vector Columns

Vector columns are automatically exposed in the REST API:

```bash
# Get documents with embeddings
curl "localhost:3000/documents?select=id,title,embedding"
```

### Vector Similarity Search

Use the `order` parameter with vector distance operators:

```bash
# Find 10 most similar documents to a query vector
curl -X POST "localhost:3000/rpc/search_similar" \
  -H "Content-Type: application/json" \
  -d '{
    "query_embedding": [0.1, 0.2, 0.3, ...],
    "match_count": 10
  }'
```

### PostgreSQL Function for Similarity Search

Create a function to encapsulate similarity search:

```sql
CREATE OR REPLACE FUNCTION search_similar(
  query_embedding vector(1536),
  match_count int DEFAULT 10,
  match_threshold float DEFAULT 0.8
)
RETURNS TABLE (
  id int,
  title text,
  content text,
  similarity float
)
LANGUAGE sql STABLE
AS $$
  SELECT
    documents.id,
    documents.title,
    documents.content,
    1 - (documents.embedding <=> query_embedding) AS similarity
  FROM documents
  WHERE 1 - (documents.embedding <=> query_embedding) > match_threshold
  ORDER BY documents.embedding <=> query_embedding
  LIMIT match_count;
$$;
```

Call via REST:

```bash
curl -X POST "localhost:3000/rpc/search_similar" \
  -H "Content-Type: application/json" \
  -d '{
    "query_embedding": [0.1, 0.2, ...],
    "match_count": 5,
    "match_threshold": 0.7
  }'
```

## GraphQL API

### Queries with Vector Fields

```graphql
query {
  documents {
    id
    title
    content
    embedding
  }
}
```

### Similarity Search via RPC

```graphql
query SearchSimilar($embedding: [Float!]!, $limit: Int) {
  searchSimilar(queryEmbedding: $embedding, matchCount: $limit) {
    id
    title
    content
    similarity
  }
}
```

## Building a RAG Pipeline

### 1. Store Embeddings

Generate embeddings using OpenAI, Cohere, or other providers, then store them:

```bash
# Insert document with embedding
curl -X POST "localhost:3000/documents" \
  -H "Content-Type: application/json" \
  -H "Prefer: return=representation" \
  -d '{
    "title": "Introduction to Rust",
    "content": "Rust is a systems programming language...",
    "embedding": [0.1, 0.2, 0.3, ...]
  }'
```

### 2. Create Search Function

```sql
CREATE OR REPLACE FUNCTION semantic_search(
  query_text text,
  query_embedding vector(1536),
  match_count int DEFAULT 5
)
RETURNS TABLE (
  id int,
  title text,
  content text,
  similarity float
)
LANGUAGE sql STABLE
AS $$
  SELECT
    id,
    title,
    content,
    1 - (embedding <=> query_embedding) AS similarity
  FROM documents
  WHERE embedding IS NOT NULL
  ORDER BY embedding <=> query_embedding
  LIMIT match_count;
$$;
```

### 3. Query for Context

```bash
# Get relevant context for LLM
curl -X POST "localhost:3000/rpc/semantic_search" \
  -H "Content-Type: application/json" \
  -d '{
    "query_text": "How do I handle errors in Rust?",
    "query_embedding": [0.1, 0.2, ...],
    "match_count": 3
  }'
```

### 4. Custom Route for Full RAG

Add a custom route for the complete RAG pipeline:

```rust
// crates/postrust-server/src/custom.rs
use axum::{extract::State, Json, routing::post};

#[derive(Deserialize)]
struct RagRequest {
    question: String,
}

#[derive(Serialize)]
struct RagResponse {
    answer: String,
    sources: Vec<Source>,
}

async fn rag_query(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RagRequest>,
) -> impl IntoResponse {
    // 1. Generate embedding for the question
    let embedding = generate_embedding(&req.question).await?;

    // 2. Search for relevant documents
    let docs = sqlx::query_as::<_, Document>(
        "SELECT * FROM semantic_search($1, $2, 5)"
    )
        .bind(&req.question)
        .bind(&embedding)
        .fetch_all(&state.pool)
        .await?;

    // 3. Build context and call LLM
    let context = docs.iter()
        .map(|d| d.content.clone())
        .collect::<Vec<_>>()
        .join("\n\n");

    let answer = call_llm(&req.question, &context).await?;

    Json(RagResponse {
        answer,
        sources: docs.into_iter().map(|d| Source {
            id: d.id,
            title: d.title,
            similarity: d.similarity,
        }).collect(),
    })
}
```

## Vector Distance Operators

pgvector supports multiple distance operators:

| Operator | Description | Use Case |
|----------|-------------|----------|
| `<->` | L2 distance (Euclidean) | General purpose |
| `<#>` | Negative inner product | When vectors are normalized |
| `<=>` | Cosine distance | Text embeddings (most common) |

## Index Types

| Index | Pros | Cons |
|-------|------|------|
| `ivfflat` | Faster build, less memory | Slightly less accurate |
| `hnsw` | More accurate, faster queries | Slower build, more memory |

### Creating Indexes

```sql
-- IVFFlat (good for most use cases)
CREATE INDEX ON documents
  USING ivfflat (embedding vector_cosine_ops)
  WITH (lists = 100);

-- HNSW (better accuracy, slower to build)
CREATE INDEX ON documents
  USING hnsw (embedding vector_cosine_ops)
  WITH (m = 16, ef_construction = 64);
```

## Performance Tips

1. **Batch inserts**: Insert embeddings in batches for better performance
2. **Appropriate dimensions**: Use the smallest embedding dimension that meets your accuracy needs
3. **Index tuning**: Adjust `lists` (ivfflat) or `m` (hnsw) based on dataset size
4. **Partial indexes**: Create indexes on frequently queried subsets

```sql
-- Partial index for active documents only
CREATE INDEX ON documents
  USING ivfflat (embedding vector_cosine_ops)
  WHERE status = 'active';
```

## Example: Product Recommendations

```sql
-- Find similar products
CREATE OR REPLACE FUNCTION recommend_products(
  product_id int,
  limit_count int DEFAULT 5
)
RETURNS SETOF products
LANGUAGE sql STABLE
AS $$
  SELECT p.*
  FROM products p, products target
  WHERE target.id = product_id
    AND p.id != product_id
    AND p.embedding IS NOT NULL
  ORDER BY p.embedding <=> target.embedding
  LIMIT limit_count;
$$;
```

```bash
# Get recommendations for product #42
curl -X POST "localhost:3000/rpc/recommend_products" \
  -d '{"product_id": 42, "limit_count": 5}'
```

## Next Steps

- See [Custom Routes](./custom-routes.md) for building complete AI pipelines
- See [API Reference](./api-reference.md) for full REST API documentation
- See [GraphQL](./graphql.md) for GraphQL-specific features
