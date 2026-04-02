# ADR-015: Fix filesystem broker symlink jail for write operations

**Status:** Accepted (implemented)
**Date:** 2026-04-23

## Context

The filesystem broker (`gang-ros/src/filesystem.rs`) implements a symlink jail to prevent path traversal attacks. When a WASM component requests file access, `check_access()` canonicalizes the path to resolve symlinks before checking it against allowed patterns.

However, there is a gap: when writing to a **new file** (one that doesn't exist yet), `Path::exists()` returns false and `canonicalize()` cannot be called. The current code falls back to using the raw, uncanonicalized path string:

```rust
let canonical = if Path::new(path).exists() {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string())
} else {
    path.to_string()  // BUG: no parent canonicalization
};
```

An attacker could request a write to `/allowed/dir/../../etc/passwd` — the file doesn't exist, so the path is not canonicalized, but the glob pattern might match the uncanonicalized string.

## Decision

Fix `check_access()` to canonicalize the parent directory when the target file does not exist:

1. Split the path into parent directory and filename
2. Canonicalize the parent (which must exist for the write to succeed)
3. Rejoin the canonical parent with the filename
4. If the parent also doesn't exist, deny the request (cannot create files in nonexistent directories)

## Consequences

- **Positive:** Closes the symlink jail bypass for write-to-new-file operations.
- **Positive:** No API change — existing policy rules continue to work.
- **Negative:** Slightly more complex path resolution logic. Must be tested with edge cases (deeply nested new paths, parent is a symlink).
- **Testing:** Add test cases for: symlink in parent to outside jail, `../` traversal in new file path, write to nonexistent parent directory.
