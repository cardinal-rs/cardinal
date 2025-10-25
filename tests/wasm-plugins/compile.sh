#!/usr/bin/env bash
set -euo pipefail

SRC="${SRC:-test1.ts}"
OUT="${OUT:-plugin.wasm}"

# Ensure AssemblyScript is available
if ! command -v npx &>/dev/null; then
  echo "❌ npx is not installed. Please install Node.js and npm." >&2
  exit 1
fi

echo "🔨 Compiling $SRC ..."
npx asc "$SRC" \
  -o "$OUT" \
  --optimize \
  --exportRuntime \
  --validate \
  2>&1

if [ -f "$OUT" ]; then
  SIZE=$(stat -c%s "$OUT" 2>/dev/null || stat -f%z "$OUT")
  echo "✅ Build succeeded: $OUT (${SIZE} bytes)"
else
  echo "❌ Build failed: $OUT not created" >&2
  exit 1
fi
