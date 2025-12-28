import { component$ } from "@builder.io/qwik";
import type { DocumentHead } from "@builder.io/qwik-city";
import { Link } from "@builder.io/qwik-city";

export default component$(() => {
  return (
    <div class="min-h-screen bg-white">
      <div class="bg-gradient-to-b from-neutral-50 to-white border-b border-neutral-200">
        <div class="container-wide py-12">
          <div class="flex items-center gap-2 text-sm text-neutral-500 mb-4">
            <Link href="/docs" class="hover:text-primary-600">Docs</Link>
            <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
            </svg>
            <span class="text-neutral-900">pgvector</span>
          </div>
          <h1 class="text-4xl font-bold text-neutral-900 mb-4">pgvector Integration</h1>
          <p class="text-lg text-neutral-600 max-w-2xl">
            Native vector similarity search for AI-powered applications, semantic search, and RAG pipelines.
          </p>
        </div>
      </div>

      <div class="container-wide py-12">
        <div class="max-w-3xl">
          {/* Overview */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Overview</h2>
            <p class="text-neutral-600 mb-4">
              Postrust provides native support for pgvector, enabling vector similarity search directly through
              your REST and GraphQL APIs. Build AI-powered applications with:
            </p>
            <ul class="list-disc list-inside text-neutral-600 space-y-2">
              <li>Semantic search using embeddings</li>
              <li>Product recommendations</li>
              <li>RAG (Retrieval Augmented Generation) pipelines</li>
              <li>Image similarity search</li>
              <li>Document clustering</li>
            </ul>
          </section>

          {/* Setup */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Setup</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">SQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`-- Enable pgvector extension
CREATE EXTENSION IF NOT EXISTS vector;

-- Create table with vector column
CREATE TABLE documents (
  id SERIAL PRIMARY KEY,
  title TEXT NOT NULL,
  content TEXT,
  embedding vector(1536)  -- OpenAI dimensions
);

-- Create index for fast similarity search
CREATE INDEX ON documents
  USING ivfflat (embedding vector_cosine_ops)
  WITH (lists = 100);`}</code>
              </pre>
            </div>
          </section>

          {/* REST API */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">REST API</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">cURL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Insert document with embedding
curl -X POST "localhost:3000/documents" \\
  -H "Content-Type: application/json" \\
  -d '{
    "title": "Introduction to Rust",
    "content": "Rust is a systems programming language...",
    "embedding": [0.1, 0.2, 0.3, ...]
  }'

# Search similar documents via RPC
curl -X POST "localhost:3000/rpc/search_similar" \\
  -H "Content-Type: application/json" \\
  -d '{
    "query_embedding": [0.1, 0.2, ...],
    "match_count": 10
  }'`}</code>
              </pre>
            </div>
          </section>

          {/* Similarity Search Function */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Similarity Search Function</h2>
            <p class="text-neutral-600 mb-4">
              Create a PostgreSQL function for semantic search:
            </p>
            <div class="bg-neutral-900 rounded-xl overflow-hidden mb-4">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">SQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`CREATE OR REPLACE FUNCTION search_similar(
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
$$;`}</code>
              </pre>
            </div>
          </section>

          {/* GraphQL */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">GraphQL API</h2>
            <div class="bg-neutral-900 rounded-xl overflow-hidden">
              <div class="px-4 py-2 bg-neutral-800 border-b border-neutral-700">
                <span class="text-sm text-neutral-400">GraphQL</span>
              </div>
              <pre class="p-4 text-sm overflow-x-auto">
                <code class="text-neutral-100">{`# Query documents with embeddings
query {
  documents {
    id
    title
    embedding
  }
}

# Similarity search via RPC
query SearchSimilar($embedding: [Float!]!) {
  searchSimilar(queryEmbedding: $embedding, matchCount: 10) {
    id
    title
    content
    similarity
  }
}`}</code>
              </pre>
            </div>
          </section>

          {/* Distance Operators */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Distance Operators</h2>
            <div class="overflow-x-auto">
              <table class="w-full text-sm">
                <thead>
                  <tr class="border-b border-neutral-200">
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Operator</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Description</th>
                    <th class="text-left py-3 px-4 font-semibold text-neutral-900">Use Case</th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-neutral-100">
                  {[
                    { op: "<->", desc: "L2 distance (Euclidean)", use: "General purpose" },
                    { op: "<#>", desc: "Negative inner product", use: "Normalized vectors" },
                    { op: "<=>", desc: "Cosine distance", use: "Text embeddings" },
                  ].map((row) => (
                    <tr key={row.op}>
                      <td class="py-3 px-4 font-mono text-primary-600">{row.op}</td>
                      <td class="py-3 px-4 text-neutral-600">{row.desc}</td>
                      <td class="py-3 px-4 text-neutral-600">{row.use}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>

          {/* Index Types */}
          <section class="mb-12">
            <h2 class="text-2xl font-bold text-neutral-900 mb-4">Index Types</h2>
            <div class="grid md:grid-cols-2 gap-4">
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">IVFFlat</h3>
                <p class="text-sm text-neutral-600 mb-2">Faster build, less memory. Good for most use cases.</p>
                <code class="text-xs text-primary-600">USING ivfflat (embedding vector_cosine_ops)</code>
              </div>
              <div class="p-4 bg-neutral-50 rounded-lg">
                <h3 class="font-semibold text-neutral-900 mb-2">HNSW</h3>
                <p class="text-sm text-neutral-600 mb-2">More accurate, faster queries. Slower to build.</p>
                <code class="text-xs text-primary-600">USING hnsw (embedding vector_cosine_ops)</code>
              </div>
            </div>
          </section>

          {/* Next */}
          <div class="flex items-center justify-between pt-8 border-t border-neutral-200">
            <Link
              href="/docs/graphql"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7"/>
              </svg>
              GraphQL
            </Link>
            <Link
              href="/docs/realtime"
              class="flex items-center gap-2 text-neutral-600 hover:text-primary-600"
            >
              Realtime Subscriptions
              <svg class="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7"/>
              </svg>
            </Link>
          </div>
        </div>
      </div>
    </div>
  );
});

export const head: DocumentHead = {
  title: "pgvector Integration - Postrust Documentation",
  meta: [
    {
      name: "description",
      content: "Native vector similarity search with pgvector for AI-powered applications, semantic search, and RAG pipelines.",
    },
  ],
};
