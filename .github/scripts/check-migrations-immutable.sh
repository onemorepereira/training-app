#!/bin/sh
# Fail if any shipped migration (src-tauri/migrations/NNNN_*.sql) is modified,
# deleted or renamed between the merge base with <base-ref> and HEAD.
# Adding new migration files is allowed.
#
# Renames are reported as delete + add (--no-renames), so renaming a shipped
# file is caught as a deletion. The legacy 3-digit files (001_init.sql ...
# 008_work_and_vi.sql) predate the migration runner and were retired once when
# it was introduced; they are outside the 4-digit pattern and not guarded.
#
# usage: check-migrations-immutable.sh <base-ref>   e.g. origin/main
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <base-ref>" >&2
    exit 2
fi
base="$1"

changed=$(git diff --no-renames --diff-filter=MD --name-only "$base...HEAD" -- src-tauri/migrations/ \
    | grep -E '^src-tauri/migrations/[0-9]{4}_[^/]*\.sql$' || true)

if [ -n "$changed" ]; then
    echo "::error::Shipped migrations must never be edited, deleted or renamed. Add a new numbered migration instead."
    echo "$changed"
    exit 1
fi
echo "No shipped migration was modified or deleted."
