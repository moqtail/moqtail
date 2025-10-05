# Contributing to MOQtail

Thank you for your interest in contributing to MOQtail! This project welcomes contributions of all kinds, including code, documentation, tests, and bug reports.

## Project Structure

MOQtail is a monorepo with Rust and TypeScript libraries and applications:

- `libs/moqtail-ts/`: TypeScript MoQ protocol library
- `libs/moqtail-rs/`: Rust MoQ protocol library
- `apps/`: Example applications (client, relay)
- `scripts/`: Project automation scripts

## Development Guidelines

- **TypeScript**: Use type-safe patterns and prefer immutability.
- **Rust**: Follow idiomatic Rust practices and format code with `rustfmt` and `clippy`.
- **Testing**: Add or update tests for new features and bug fixes. Use Vitest for TypeScript and standard Rust test framework for Rust.
- **Documentation**: Update or add documentation as needed, especially for public APIs.
- **Commits**: Follow Conventional Commits. Husky will install `prepare-commit-msg` hook to enforce commit message format.
- **Pre-commit**: The project uses Husky and lint-staged. Pre-commit hooks will run formatting and linting.
- **Changesets**: If you make a user-facing change, you will be prompted to add a changeset before commit.

## How to Contribute

1. **Fork the repository** and create your branch from `main`.
2. **Install dependencies**: `npm install`
3. **Build and test**:
   - TypeScript: `npm run build` and `npm run test`
   - Rust: `cargo build` and `cargo test` in the relevant directory
4. **Run linters and formatters**: `npm run format` (or for Rust, `cargo fmt`)
5. **Open a Pull Request** with a clear description of your changes.
6. **Add a changeset** if your changes are user-facing. This will help with versioning and changelog generation.

## Testing

- TypeScript: Use Vitest. Place tests inline or alongside source files.
- Rust: Use the standard test framework. Place tests inline.

## Commit Message Format

This project uses [Conventional Commits](https://www.conventionalcommits.org/). Example:

```
feat(control): add SubscribeNamespace and SubscribeDone message support
fix(subscribe): correct endGroup validation for AbsoluteRange
```

> [!NOTE]
> The commit message format is purely for consistency and clarity. Changelogs and versioning will be handled by changesets.

## Changesets

Changesets are used to manage versioning and changelogs. When you make a user-facing change, you will be prompted to create a changeset. This helps keep track of changes and ensures proper versioning. Usually, the `pre-commit` hook will prompt you to create a changeset every time you commit something. If you wish to create a changeset manually, you can run:

```bash
npm run changeset
```

Remember, not every pull request requires a changeset—only those that introduce user-facing changes affecting the API or functionality. For internal refactoring, tests, or documentation updates, a changeset is not needed.

---

For any questions, open an issue or start a discussion. Happy coding!
