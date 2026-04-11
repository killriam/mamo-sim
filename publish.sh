#!/usr/bin/env bash
# publish.sh — build, version-bump, tag, and publish @killriam/mamo-sim
#
# Usage:
#   ./publish.sh patch   # 0.1.0 → 0.1.1
#   ./publish.sh minor   # 0.1.0 → 0.2.0
#   ./publish.sh major   # 0.1.0 → 1.0.0

set -euo pipefail

BUMP="${1:-patch}"

if [[ "$BUMP" != "patch" && "$BUMP" != "minor" && "$BUMP" != "major" ]]; then
    echo "Usage: $0 [patch|minor|major]"
    exit 1
fi

# ── 1. Tests ─────────────────────────────────────────────────────────────────
echo "▶ Running tests..."
cargo test --all-features

# ── 2. Bump version in Cargo.toml ────────────────────────────────────────────
echo "▶ Bumping version ($BUMP)..."

CURRENT=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT"

case "$BUMP" in
    major) MAJOR=$((MAJOR + 1)); MINOR=0; PATCH=0 ;;
    minor) MINOR=$((MINOR + 1)); PATCH=0 ;;
    patch) PATCH=$((PATCH + 1)) ;;
esac

NEW_VERSION="$MAJOR.$MINOR.$PATCH"
sed -i "s/^version = \"$CURRENT\"/version = \"$NEW_VERSION\"/" Cargo.toml

echo "   $CURRENT → $NEW_VERSION"

# ── 3. Build WASM package ────────────────────────────────────────────────────
echo "▶ Building WASM..."
wasm-pack build --target web --scope killriam --release

# ── 4. Publish to npm ────────────────────────────────────────────────────────
echo "▶ Publishing @killriam/mamo-sim@$NEW_VERSION to npm..."
cd pkg
npm publish --access public
cd ..

# ── 5. Commit + tag ─────────────────────────────────────────────────────────
echo "▶ Committing and tagging v$NEW_VERSION..."
git add Cargo.toml Cargo.lock
git commit -m "chore: release v$NEW_VERSION"
git tag "v$NEW_VERSION"

echo ""
echo "✓ Published @killriam/mamo-sim@$NEW_VERSION"
echo "  Push with: git push && git push --tags"
echo ""
echo "  Then update MaMoFrontend:"
echo "    npm install @killriam/mamo-sim@$NEW_VERSION"
