# Databases folder (plain-file dumps)

DevStack keeps a user-visible **`Databases/`** folder of portable dumps — not raw engine data directories.

## Why dumps, not raw files

Copying InnoDB / Postgres / WiredTiger files is unsafe and non-portable:

- files can be mid-write while the server runs
- dumps require matching major versions and identical `my.cnf` / `postgresql.conf`
- restoring raw files needs a full clean shutdown and exclusive access

Instead we stream **`mysqldump` / `pg_dump` / `mongodump`** into gzip archives that import anywhere.

## Layout

```
{data}/Databases/          # or a custom path from General settings
  mysql/{db}.sql.gz
  mariadb/{db}.sql.gz
  postgres/{db}.sql.gz
  mongo/{db}.archive.gz
  */.safety/               # pre-import backups
```

## Auto-export

- **On stop** — when MariaDB/MySQL/Postgres/Mongo stop (if enabled)
- **Interval** — every N seconds while engines run (`0` disables)
- **On demand** — Export all from General, or per-DB

## Folder watcher

Dropping/replacing a dump under an engine folder:

- **`ask`** (default) — UI can confirm before import
- **`auto`** — stream-import immediately (after a safety dump)

## Site Download / Import

Site detail:

- **Download database** — streamed dump to a chosen path
- **Import database** — `.sql` / `.sql.gz` / `.sql.zst` / `.zip`

WordPress imports auto-detect the old `siteurl` and run serialized-safe `wp search-replace` (http/https + domain variants). Other sites can pass optional search→replace strings.
