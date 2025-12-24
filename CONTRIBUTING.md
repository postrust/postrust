# Contributing to Postrust

Thank you for your interest in contributing to Postrust! This document provides guidelines and information for contributors.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Pull Request Process](#pull-request-process)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Documentation](#documentation)

## Code of Conduct

This project adheres to a code of conduct. By participating, you are expected to uphold this code. Please be respectful and constructive in all interactions.

## Getting Started

### Prerequisites

- Rust 1.75 or later
- Docker and Docker Compose (for running tests with PostgreSQL)
- Git

### Fork and Clone

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/postrust.git
   cd postrust
   ```
3. Add the upstream remote:
   ```bash
   git remote add upstream https://github.com/postrust/postrust.git
   ```

## Development Setup

### Quick Start

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the project
cargo build

# Run tests
cargo test

# Run with Docker (includes PostgreSQL)
docker-compose up -d postgres
docker-compose run test
```

### Running the Server Locally

```bash
# Start PostgreSQL
docker-compose up -d postgres

# Set environment variables
export DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test"
export PGRST_DB_ANON_ROLE="web_anon"
export PGRST_JWT_SECRET="your-secret-key"

# Run the server
cargo run -p postrust-server
```

### IDE Setup

We recommend using VS Code or RustRover with the following extensions:
- rust-analyzer
- Even Better TOML
- crates

## Making Changes

### Branch Naming

Use descriptive branch names:
- `feature/add-graphql-support`
- `fix/jwt-validation-error`
- `docs/update-api-examples`
- `refactor/query-builder`

### Commit Messages

Follow conventional commits format:

```
type(scope): description

[optional body]

[optional footer]
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

Examples:
```
feat(auth): add support for RS256 JWT algorithm
fix(query): handle null values in IN operator
docs(readme): add AWS Lambda deployment example
```

## Pull Request Process

1. **Create a feature branch** from `main`:
   ```bash
   git checkout -b feature/your-feature
   ```

2. **Make your changes** and commit them with descriptive messages

3. **Ensure tests pass**:
   ```bash
   cargo test --all
   cargo clippy --all -- -D warnings
   cargo fmt --all -- --check
   ```

4. **Update documentation** if needed

5. **Push your branch** and create a Pull Request

6. **Address review feedback** and update your PR as needed

### PR Checklist

- [ ] Tests added/updated for new functionality
- [ ] Documentation updated (README, doc comments)
- [ ] Commit messages follow conventions
- [ ] All tests pass locally
- [ ] No clippy warnings
- [ ] Code formatted with `cargo fmt`

## Coding Standards

### Rust Style

- Follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Document public APIs with doc comments

### Code Organization

```
crates/
├── postrust-core/      # Core library
│   ├── api_request/    # Request parsing
│   ├── schema_cache/   # Database introspection
│   ├── plan/           # Query planning
│   └── query/          # SQL generation
├── postrust-sql/       # SQL builder
├── postrust-auth/      # Authentication
├── postrust-response/  # Response formatting
├── postrust-server/    # HTTP server
├── postrust-lambda/    # Lambda adapter
└── postrust-worker/    # Workers adapter
```

### Error Handling

- Use `thiserror` for error types
- Provide meaningful error messages
- Include context in error chains
- Map database errors to appropriate HTTP status codes

### SQL Safety

- **Always** use parameterized queries
- Never concatenate user input into SQL strings
- Use `escape_ident` for identifiers
- Use `SqlParam` for values

```rust
// Good
frag.push_param(user_input);

// Bad - Never do this!
frag.push(&format!("'{}'", user_input));
```

## Testing

### Running Tests

```bash
# All tests
cargo test --all

# Specific crate
cargo test -p postrust-core

# With output
cargo test -- --nocapture

# With coverage (requires cargo-tarpaulin)
cargo tarpaulin --out Html
```

### Writing Tests

- Place unit tests in the same file as the code
- Use `#[cfg(test)]` module
- Test both success and error cases
- Use descriptive test names

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_filter() {
        let result = parse_filter("name=eq.John").unwrap();
        assert_eq!(result.field, "name");
        assert_eq!(result.operator, "eq");
    }

    #[test]
    fn test_parse_invalid_filter_returns_error() {
        let result = parse_filter("invalid");
        assert!(result.is_err());
    }
}
```

### Integration Tests

Integration tests requiring a database should:
- Use Docker Compose to start PostgreSQL
- Use the `scripts/init-db.sql` schema
- Clean up after themselves

```bash
# Run integration tests
docker-compose up -d postgres
DATABASE_URL="postgres://postgres:postgres@localhost:5432/postrust_test" cargo test
```

## Documentation

### Code Documentation

- Document all public APIs
- Include examples in doc comments
- Use `///` for item docs, `//!` for module docs

```rust
/// Parse a filter expression from a query parameter.
///
/// # Arguments
///
/// * `input` - The filter string (e.g., "name=eq.John")
///
/// # Returns
///
/// Returns a `Filter` on success, or an error if parsing fails.
///
/// # Examples
///
/// ```
/// let filter = parse_filter("age=gt.21").unwrap();
/// assert_eq!(filter.operator, "gt");
/// ```
pub fn parse_filter(input: &str) -> Result<Filter, ParseError> {
    // ...
}
```

### README and Guides

- Keep README.md up to date
- Add examples for new features
- Update configuration docs for new options

## Questions?

- Open a [GitHub Issue](https://github.com/postrust/postrust/issues) for bugs or feature requests
- Start a [Discussion](https://github.com/postrust/postrust/discussions) for questions

Thank you for contributing!
