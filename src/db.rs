use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RepoMeta {
    pub path: String,
    pub name: String,
    pub default_branch: Option<String>,
    pub last_commit_ts: Option<i64>,
    pub last_scan_ts: i64,
    pub readme_excerpt: Option<String>,
    pub origin_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub default_branch: Option<String>,
    pub last_commit_ts: Option<i64>,
    pub last_scan_ts: i64,
    pub readme_excerpt: Option<String>,
    pub origin_url: Option<String>,
    pub last_access_ts: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct RepoWithTags {
    pub repo: RepoRow,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Paged<T> {
    pub total: usize,
    pub items: Vec<T>,
}

#[derive(Debug, Clone)]
pub struct CommitBranch {
    pub kind: String,
    pub name: String,
    pub refname: String,
    pub tip_time: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CommitIndexRow {
    pub refname: String,
    pub branch_kind: String,
    pub branch_name: String,
    pub oid: String,
    pub time: Option<i64>,
    pub author: Option<String>,
    pub email: Option<String>,
    pub summary: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommitHit {
    pub repo_name: String,
    pub repo_path: String,
    pub branch_kind: String,
    pub branch_name: String,
    pub refname: String,
    pub oid: String,
    pub time: Option<i64>,
    pub summary: Option<String>,
    pub message: Option<String>,
}

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self { conn })
    }

    pub fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS repos (
              id              INTEGER PRIMARY KEY AUTOINCREMENT,
              path            TEXT NOT NULL UNIQUE,
              name            TEXT NOT NULL,
              default_branch  TEXT,
              last_commit_ts  INTEGER,
              last_scan_ts    INTEGER NOT NULL,
              readme_excerpt  TEXT,
              origin_url      TEXT,
              last_access_ts  INTEGER
            );

            CREATE TABLE IF NOT EXISTS tags (
              id    INTEGER PRIMARY KEY AUTOINCREMENT,
              name  TEXT NOT NULL UNIQUE
            );

            CREATE TABLE IF NOT EXISTS repo_tags (
              repo_id INTEGER NOT NULL,
              tag_id  INTEGER NOT NULL,
              PRIMARY KEY (repo_id, tag_id),
              FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE,
              FOREIGN KEY (tag_id)  REFERENCES tags(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_repos_name ON repos(name);
            CREATE INDEX IF NOT EXISTS idx_repos_access ON repos(last_access_ts);
            CREATE INDEX IF NOT EXISTS idx_tags_name ON tags(name);

            CREATE TABLE IF NOT EXISTS commit_branches (
              id        INTEGER PRIMARY KEY AUTOINCREMENT,
              repo_id   INTEGER NOT NULL,
              kind      TEXT NOT NULL,
              name      TEXT NOT NULL,
              refname   TEXT NOT NULL,
              tip_time  INTEGER,
              UNIQUE(repo_id, refname),
              FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS commits (
              id          INTEGER PRIMARY KEY AUTOINCREMENT,
              repo_id     INTEGER NOT NULL,
              refname     TEXT NOT NULL,
              branch_kind TEXT NOT NULL,
              branch_name TEXT NOT NULL,
              oid         TEXT NOT NULL,
              time        INTEGER,
              author      TEXT,
              email       TEXT,
              summary     TEXT,
              message     TEXT,
              UNIQUE(repo_id, refname, oid),
              FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_commits_repo_time ON commits(repo_id, time);
            CREATE INDEX IF NOT EXISTS idx_commits_repo_ref_time ON commits(repo_id, refname, time);
            CREATE INDEX IF NOT EXISTS idx_commits_branch_name ON commits(branch_name);
            "#,
        )?;
        // Schema migration for older DBs (SQLite has no IF NOT EXISTS for ADD COLUMN).
        let _ = self.conn.execute("ALTER TABLE repos ADD COLUMN origin_url TEXT", []);
        Ok(())
    }

    pub fn upsert_repo(&self, meta: &RepoMeta) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO repos (path, name, default_branch, last_commit_ts, last_scan_ts, readme_excerpt, origin_url)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(path) DO UPDATE SET
              name = excluded.name,
              default_branch = excluded.default_branch,
              last_commit_ts = excluded.last_commit_ts,
              last_scan_ts = excluded.last_scan_ts,
              readme_excerpt = excluded.readme_excerpt,
              origin_url = excluded.origin_url
            "#,
            params![
                meta.path,
                meta.name,
                meta.default_branch,
                meta.last_commit_ts,
                meta.last_scan_ts,
                meta.readme_excerpt,
                meta.origin_url
            ],
        )?;
        Ok(())
    }

    pub fn list_repos(&self, tag: Option<&str>, recent: bool) -> Result<Vec<RepoRow>> {
        let mut rows = Vec::new();
        if let Some(tag) = tag {
            let order = if recent {
                "ORDER BY COALESCE(r.last_access_ts, 0) DESC, r.name ASC"
            } else {
                "ORDER BY r.name ASC"
            };
            let sql = format!(
                r#"
                SELECT r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts
                FROM repos r
                JOIN repo_tags rt ON rt.repo_id = r.id
                JOIN tags t ON t.id = rt.tag_id
                WHERE t.name = ?1
                {order}
                "#
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let iter = stmt.query_map([tag], |r| {
                Ok(RepoRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    default_branch: r.get(3)?,
                    last_commit_ts: r.get(4)?,
                    last_scan_ts: r.get(5)?,
                    readme_excerpt: r.get(6)?,
                    origin_url: r.get(7)?,
                    last_access_ts: r.get(8)?,
                })
            })?;
            for r in iter {
                rows.push(r?);
            }
        } else {
            let order = if recent {
                "ORDER BY COALESCE(last_access_ts, 0) DESC, name ASC"
            } else {
                "ORDER BY name ASC"
            };
            let sql = format!(
                "SELECT id, path, name, default_branch, last_commit_ts, last_scan_ts, readme_excerpt, origin_url, last_access_ts FROM repos {order}"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let iter = stmt.query_map([], |r| {
                Ok(RepoRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    default_branch: r.get(3)?,
                    last_commit_ts: r.get(4)?,
                    last_scan_ts: r.get(5)?,
                    readme_excerpt: r.get(6)?,
                    origin_url: r.get(7)?,
                    last_access_ts: r.get(8)?,
                })
            })?;
            for r in iter {
                rows.push(r?);
            }
        }
        Ok(rows)
    }

    pub fn list_repos_with_tags(&self, tag: Option<&str>, recent: bool) -> Result<Vec<RepoWithTags>> {
        let order = if recent {
            "ORDER BY COALESCE(r.last_access_ts, 0) DESC, r.name ASC"
        } else {
            "ORDER BY r.name ASC"
        };

        let (sql, args): (String, Vec<String>) = if let Some(tag) = tag {
            (
                format!(
                    r#"
                    SELECT
                      r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts,
                      COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
                    FROM repos r
                    LEFT JOIN repo_tags rt ON rt.repo_id = r.id
                    LEFT JOIN tags t ON t.id = rt.tag_id
                    WHERE EXISTS (
                      SELECT 1 FROM repo_tags rt2
                      JOIN tags t2 ON t2.id = rt2.tag_id
                      WHERE rt2.repo_id = r.id AND t2.name = ?1
                    )
                    GROUP BY r.id
                    {order}
                    "#
                ),
                vec![tag.to_string()],
            )
        } else {
            (
                format!(
                    r#"
                    SELECT
                      r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts,
                      COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
                    FROM repos r
                    LEFT JOIN repo_tags rt ON rt.repo_id = r.id
                    LEFT JOIN tags t ON t.id = rt.tag_id
                    GROUP BY r.id
                    {order}
                    "#
                ),
                vec![],
            )
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = Vec::new();
        if args.is_empty() {
            let iter = stmt.query_map([], |r| {
                Ok((
                    RepoRow {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        name: r.get(2)?,
                        default_branch: r.get(3)?,
                        last_commit_ts: r.get(4)?,
                        last_scan_ts: r.get(5)?,
                        readme_excerpt: r.get(6)?,
                        origin_url: r.get(7)?,
                        last_access_ts: r.get(8)?,
                    },
                    r.get::<_, String>(9)?,
                ))
            })?;
            for row in iter {
                let (repo, tags) = row?;
                let tags = tags
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                rows.push(RepoWithTags { repo, tags });
            }
        } else {
            let iter = stmt.query_map([args[0].as_str()], |r| {
                Ok((
                    RepoRow {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        name: r.get(2)?,
                        default_branch: r.get(3)?,
                        last_commit_ts: r.get(4)?,
                        last_scan_ts: r.get(5)?,
                        readme_excerpt: r.get(6)?,
                        origin_url: r.get(7)?,
                        last_access_ts: r.get(8)?,
                    },
                    r.get::<_, String>(9)?,
                ))
            })?;
            for row in iter {
                let (repo, tags) = row?;
                let tags = tags
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                rows.push(RepoWithTags { repo, tags });
            }
        }
        Ok(rows)
    }

    pub fn list_repos_with_tags_paged(
        &self,
        tag: Option<&str>,
        recent: bool,
        page: usize,
        per_page: usize,
    ) -> Result<Paged<RepoWithTags>> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 200);
        let offset = (page - 1) * per_page;

        let total: usize = if let Some(tag) = tag {
            self.conn.query_row(
                r#"
                SELECT COUNT(*)
                FROM repos r
                WHERE EXISTS (
                  SELECT 1 FROM repo_tags rt2
                  JOIN tags t2 ON t2.id = rt2.tag_id
                  WHERE rt2.repo_id = r.id AND t2.name = ?1
                )
                "#,
                [tag],
                |r| r.get::<_, i64>(0),
            )? as usize
        } else {
            self.conn
                .query_row("SELECT COUNT(*) FROM repos", [], |r| r.get::<_, i64>(0))? as usize
        };

        let order = if recent {
            "ORDER BY COALESCE(r.last_access_ts, 0) DESC, r.name ASC"
        } else {
            "ORDER BY r.name ASC"
        };

        let (sql, args): (String, Vec<String>) = if let Some(tag) = tag {
            (
                format!(
                    r#"
                    SELECT
                      r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts,
                      COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
                    FROM repos r
                    LEFT JOIN repo_tags rt ON rt.repo_id = r.id
                    LEFT JOIN tags t ON t.id = rt.tag_id
                    WHERE EXISTS (
                      SELECT 1 FROM repo_tags rt2
                      JOIN tags t2 ON t2.id = rt2.tag_id
                      WHERE rt2.repo_id = r.id AND t2.name = ?1
                    )
                    GROUP BY r.id
                    {order}
                    LIMIT ?2 OFFSET ?3
                    "#
                ),
                vec![tag.to_string(), per_page.to_string(), offset.to_string()],
            )
        } else {
            (
                format!(
                    r#"
                    SELECT
                      r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts,
                      COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
                    FROM repos r
                    LEFT JOIN repo_tags rt ON rt.repo_id = r.id
                    LEFT JOIN tags t ON t.id = rt.tag_id
                    GROUP BY r.id
                    {order}
                    LIMIT ?1 OFFSET ?2
                    "#
                ),
                vec![per_page.to_string(), offset.to_string()],
            )
        };

        let mut stmt = self.conn.prepare(&sql)?;
        let mut items = Vec::new();

        if tag.is_some() {
            let iter = stmt.query_map(
                params![args[0].as_str(), per_page as i64, offset as i64],
                |r| {
                    Ok((
                        RepoRow {
                            id: r.get(0)?,
                            path: r.get(1)?,
                            name: r.get(2)?,
                            default_branch: r.get(3)?,
                            last_commit_ts: r.get(4)?,
                            last_scan_ts: r.get(5)?,
                            readme_excerpt: r.get(6)?,
                            origin_url: r.get(7)?,
                            last_access_ts: r.get(8)?,
                        },
                        r.get::<_, String>(9)?,
                    ))
                },
            )?;
            for row in iter {
                let (repo, tags) = row?;
                let tags = tags
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                items.push(RepoWithTags { repo, tags });
            }
        } else {
            let iter = stmt.query_map(params![per_page as i64, offset as i64], |r| {
                Ok((
                    RepoRow {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        name: r.get(2)?,
                        default_branch: r.get(3)?,
                        last_commit_ts: r.get(4)?,
                        last_scan_ts: r.get(5)?,
                        readme_excerpt: r.get(6)?,
                        origin_url: r.get(7)?,
                        last_access_ts: r.get(8)?,
                    },
                    r.get::<_, String>(9)?,
                ))
            })?;
            for row in iter {
                let (repo, tags) = row?;
                let tags = tags
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                items.push(RepoWithTags { repo, tags });
            }
        }

        Ok(Paged { total, items })
    }

    pub fn search_repos(&self, query: &str) -> Result<Vec<RepoRow>> {
        let q = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts
            FROM repos r
            LEFT JOIN repo_tags rt ON rt.repo_id = r.id
            LEFT JOIN tags t ON t.id = rt.tag_id
            WHERE r.name LIKE ?1 OR r.path LIKE ?1 OR COALESCE(r.readme_excerpt, '') LIKE ?1 OR COALESCE(t.name, '') LIKE ?1
            ORDER BY r.name ASC
            "#,
        )?;

        let iter = stmt.query_map([q], |r| {
            Ok(RepoRow {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                default_branch: r.get(3)?,
                last_commit_ts: r.get(4)?,
                last_scan_ts: r.get(5)?,
                readme_excerpt: r.get(6)?,
                origin_url: r.get(7)?,
                last_access_ts: r.get(8)?,
            })
        })?;

        let mut rows = Vec::new();
        for r in iter {
            rows.push(r?);
        }
        Ok(rows)
    }

    pub fn search_repos_with_tags(&self, query: &str) -> Result<Vec<RepoWithTags>> {
        let q = format!("%{}%", query);
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
              r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.last_access_ts,
              COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
            FROM repos r
            LEFT JOIN repo_tags rt ON rt.repo_id = r.id
            LEFT JOIN tags t ON t.id = rt.tag_id
            WHERE r.name LIKE ?1 OR r.path LIKE ?1 OR COALESCE(r.readme_excerpt, '') LIKE ?1 OR COALESCE(t.name, '') LIKE ?1
            GROUP BY r.id
            ORDER BY r.name ASC
            "#,
        )?;

        let iter = stmt.query_map([q], |r| {
            Ok((
                    RepoRow {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        name: r.get(2)?,
                        default_branch: r.get(3)?,
                        last_commit_ts: r.get(4)?,
                        last_scan_ts: r.get(5)?,
                        readme_excerpt: r.get(6)?,
                        origin_url: r.get(7)?,
                        last_access_ts: r.get(8)?,
                    },
                    r.get::<_, String>(9)?,
                ))
            })?;

        let mut rows = Vec::new();
        for row in iter {
            let (repo, tags) = row?;
            let tags = tags
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            rows.push(RepoWithTags { repo, tags });
        }
        Ok(rows)
    }

    pub fn search_repos_with_tags_paged(
        &self,
        query: &str,
        page: usize,
        per_page: usize,
    ) -> Result<Paged<RepoWithTags>> {
        self.search_repos_with_tags_paged_filtered(query, true, true, true, true, page, per_page)
    }

    pub fn search_repos_with_tags_paged_filtered(
        &self,
        query: &str,
        in_name: bool,
        in_path: bool,
        in_readme: bool,
        in_tags: bool,
        page: usize,
        per_page: usize,
    ) -> Result<Paged<RepoWithTags>> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 200);
        let offset = (page - 1) * per_page;
        let q = format!("%{}%", query);

        let (in_name, in_path, in_readme, in_tags) = if !(in_name || in_path || in_readme || in_tags)
        {
            (true, true, true, true)
        } else {
            (in_name, in_path, in_readme, in_tags)
        };

        let mut where_parts = Vec::<&str>::new();
        if in_name {
            where_parts.push("r.name LIKE ?1");
        }
        if in_path {
            where_parts.push("r.path LIKE ?1");
        }
        if in_readme {
            where_parts.push("COALESCE(r.readme_excerpt, '') LIKE ?1");
        }
        if in_tags {
            where_parts.push("COALESCE(t.name, '') LIKE ?1");
        }
        let where_sql = where_parts.join(" OR ");

        let total_sql = format!(
            r#"
            SELECT COUNT(DISTINCT r.id)
            FROM repos r
            LEFT JOIN repo_tags rt ON rt.repo_id = r.id
            LEFT JOIN tags t ON t.id = rt.tag_id
            WHERE {where_sql}
            "#
        );
        let total: usize = self
            .conn
            .query_row(&total_sql, [q.as_str()], |r| r.get::<_, i64>(0))? as usize;

        let sql = format!(
            r#"
            SELECT
              r.id, r.path, r.name, r.default_branch, r.last_commit_ts, r.last_scan_ts, r.readme_excerpt, r.origin_url, r.last_access_ts,
              COALESCE(GROUP_CONCAT(t.name, ','), '') AS tags
            FROM repos r
            LEFT JOIN repo_tags rt ON rt.repo_id = r.id
            LEFT JOIN tags t ON t.id = rt.tag_id
            WHERE {where_sql}
            GROUP BY r.id
            ORDER BY r.name ASC
            LIMIT ?2 OFFSET ?3
            "#
        );
        let mut stmt = self.conn.prepare(&sql)?;

        let iter = stmt.query_map(params![q, per_page as i64, offset as i64], |r| {
            Ok((
                RepoRow {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    default_branch: r.get(3)?,
                    last_commit_ts: r.get(4)?,
                    last_scan_ts: r.get(5)?,
                    readme_excerpt: r.get(6)?,
                    origin_url: r.get(7)?,
                    last_access_ts: r.get(8)?,
                },
                r.get::<_, String>(9)?,
            ))
        })?;

        let mut items = Vec::new();
        for row in iter {
            let (repo, tags) = row?;
            let tags = tags
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            items.push(RepoWithTags { repo, tags });
        }

        Ok(Paged { total, items })
    }

    pub fn add_tag_to_repo(&self, repo_path: &str, tag: &str) -> Result<()> {
        let repo_id = self
            .repo_id_by_path(repo_path)?
            .with_context(|| format!("repo not indexed: {repo_path}"))?;
        let tag_id = self.ensure_tag(tag)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO repo_tags (repo_id, tag_id) VALUES (?1, ?2)",
            params![repo_id, tag_id],
        )?;
        Ok(())
    }

    pub fn remove_tag_from_repo(&self, repo_path: &str, tag: &str) -> Result<()> {
        let repo_id = self
            .repo_id_by_path(repo_path)?
            .with_context(|| format!("repo not indexed: {repo_path}"))?;
        let tag_id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))
            .optional()?;
        let Some(tag_id) = tag_id else { return Ok(()); };
        self.conn.execute(
            "DELETE FROM repo_tags WHERE repo_id = ?1 AND tag_id = ?2",
            params![repo_id, tag_id],
        )?;
        // Remove the tag itself if it is now orphaned.
        self.conn.execute(
            "DELETE FROM tags WHERE id = ?1 AND NOT EXISTS (SELECT 1 FROM repo_tags WHERE tag_id = ?1)",
            params![tag_id],
        )?;
        Ok(())
    }

    pub fn list_tags(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM tags ORDER BY name ASC")?;
        let iter = stmt.query_map([], |r| r.get(0))?;
        let mut out = Vec::new();
        for t in iter {
            out.push(t?);
        }
        Ok(out)
    }

    pub fn list_tags_with_count(&self) -> Result<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT t.name, COUNT(rt.repo_id) AS c
            FROM tags t
            LEFT JOIN repo_tags rt ON rt.tag_id = t.id
            GROUP BY t.id
            HAVING c > 0
            ORDER BY t.name ASC
            "#,
        )?;
        let iter = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)))?;
        let mut out = Vec::new();
        for row in iter {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn list_repo_tags(&self, repo_path: &str) -> Result<Vec<String>> {
        let repo_id = self
            .repo_id_by_path(repo_path)?
            .with_context(|| format!("repo not indexed: {repo_path}"))?;
        let mut stmt = self.conn.prepare(
            r#"
            SELECT t.name
            FROM tags t
            JOIN repo_tags rt ON rt.tag_id = t.id
            WHERE rt.repo_id = ?1
            ORDER BY t.name ASC
            "#,
        )?;
        let iter = stmt.query_map([repo_id], |r| r.get(0))?;
        let mut out = Vec::new();
        for t in iter {
            out.push(t?);
        }
        Ok(out)
    }

    pub fn record_access(&self, repo_path: &str) -> Result<()> {
        let ts = chrono::Utc::now().timestamp();
        self.conn.execute(
            "UPDATE repos SET last_access_ts = ?1 WHERE path = ?2",
            params![ts, repo_path],
        )?;
        Ok(())
    }

    pub fn prune_missing_paths(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare("SELECT path FROM repos")?;
        let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut missing = Vec::new();
        for p in iter {
            let p = p?;
            if !Path::new(&p).exists() {
                missing.push(p);
            }
        }
        let mut deleted = 0usize;
        for p in missing {
            deleted += self.conn.execute("DELETE FROM repos WHERE path = ?1", [p])? as usize;
        }
        self.prune_orphan_tags()?;
        Ok(deleted)
    }

    pub fn prune_under_root(&self, root: &str, keep: &HashSet<String>) -> Result<usize> {
        let prefix = if root.ends_with(std::path::MAIN_SEPARATOR) {
            root.to_string()
        } else {
            format!("{root}{}", std::path::MAIN_SEPARATOR)
        };
        let like = format!("{prefix}%");
        let mut stmt = self.conn.prepare("SELECT path FROM repos WHERE path LIKE ?1")?;
        let iter = stmt.query_map([like], |r| r.get::<_, String>(0))?;
        let mut deleted = 0usize;
        for p in iter {
            let p = p?;
            if !keep.contains(&p) {
                deleted += self.conn.execute("DELETE FROM repos WHERE path = ?1", [p])? as usize;
            }
        }
        self.prune_orphan_tags()?;
        Ok(deleted)
    }

    pub fn list_repo_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM repos ORDER BY name ASC")?;
        let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for p in iter {
            out.push(p?);
        }
        Ok(out)
    }

    pub fn replace_commit_index_for_repo(
        &self,
        repo_path: &str,
        branches: &[CommitBranch],
        commits: &[CommitIndexRow],
    ) -> Result<()> {
        let repo_id = self
            .repo_id_by_path(repo_path)?
            .with_context(|| format!("repo not indexed: {repo_path}"))?;

        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM commit_branches WHERE repo_id = ?1", [repo_id])?;
        tx.execute("DELETE FROM commits WHERE repo_id = ?1", [repo_id])?;

        {
            let mut stmt = tx.prepare(
                "INSERT INTO commit_branches (repo_id, kind, name, refname, tip_time) VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for b in branches {
                stmt.execute(params![repo_id, b.kind, b.name, b.refname, b.tip_time])?;
            }
        }

        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO commits (repo_id, refname, branch_kind, branch_name, oid, time, author, email, summary, message)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                "#,
            )?;
            for c in commits {
                stmt.execute(params![
                    repo_id,
                    c.refname,
                    c.branch_kind,
                    c.branch_name,
                    c.oid,
                    c.time,
                    c.author,
                    c.email,
                    c.summary,
                    c.message
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn search_commits_paged(
        &self,
        query: &str,
        branch: Option<&str>,
        in_summary: bool,
        in_message: bool,
        page: usize,
        per_page: usize,
    ) -> Result<Paged<CommitHit>> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 200);
        let offset = (page - 1) * per_page;

        let q = format!("%{}%", query);
        let b = branch.map(|s| format!("%{}%", s));

        let (in_summary, in_message) = if !(in_summary || in_message) {
            (true, true)
        } else {
            (in_summary, in_message)
        };

        let mut where_parts = Vec::<&str>::new();
        if in_summary {
            where_parts.push("c.summary LIKE ?1");
        }
        if in_message {
            where_parts.push("COALESCE(c.message, '') LIKE ?1");
        }
        let where_sql = where_parts.join(" OR ");

        let total: usize = if let Some(b) = &b {
            let sql = format!(
                r#"
                SELECT COUNT(*)
                FROM commits c
                JOIN repos r ON r.id = c.repo_id
                WHERE ({where_sql})
                  AND (c.branch_name LIKE ?2 OR c.refname LIKE ?2)
                "#
            );
            self.conn
                .query_row(&sql, params![q, b], |r| r.get::<_, i64>(0))? as usize
        } else {
            let sql = format!(
                r#"
                SELECT COUNT(*)
                FROM commits c
                WHERE {where_sql}
                "#
            );
            self.conn
                .query_row(&sql, [q.as_str()], |r| r.get::<_, i64>(0))? as usize
        };

        let mut items = Vec::new();
        if let Some(b) = b {
            let sql = format!(
                r#"
                SELECT r.name, r.path, c.branch_kind, c.branch_name, c.refname, c.oid, c.time, c.summary, c.message
                FROM commits c
                JOIN repos r ON r.id = c.repo_id
                WHERE ({where_sql})
                  AND (c.branch_name LIKE ?2 OR c.refname LIKE ?2)
                ORDER BY COALESCE(c.time, 0) DESC
                LIMIT ?3 OFFSET ?4
                "#
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let iter = stmt.query_map(params![q, b, per_page as i64, offset as i64], |r| {
                Ok(CommitHit {
                    repo_name: r.get(0)?,
                    repo_path: r.get(1)?,
                    branch_kind: r.get(2)?,
                    branch_name: r.get(3)?,
                    refname: r.get(4)?,
                    oid: r.get(5)?,
                    time: r.get(6)?,
                    summary: r.get(7)?,
                    message: r.get(8)?,
                })
            })?;
            for row in iter {
                items.push(row?);
            }
        } else {
            let sql = format!(
                r#"
                SELECT r.name, r.path, c.branch_kind, c.branch_name, c.refname, c.oid, c.time, c.summary, c.message
                FROM commits c
                JOIN repos r ON r.id = c.repo_id
                WHERE {where_sql}
                ORDER BY COALESCE(c.time, 0) DESC
                LIMIT ?2 OFFSET ?3
                "#
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let iter = stmt.query_map(params![q, per_page as i64, offset as i64], |r| {
                Ok(CommitHit {
                    repo_name: r.get(0)?,
                    repo_path: r.get(1)?,
                    branch_kind: r.get(2)?,
                    branch_name: r.get(3)?,
                    refname: r.get(4)?,
                    oid: r.get(5)?,
                    time: r.get(6)?,
                    summary: r.get(7)?,
                    message: r.get(8)?,
                })
            })?;
            for row in iter {
                items.push(row?);
            }
        }

        Ok(Paged { total, items })
    }

    pub fn resolve_repo_path(&self, input: &str) -> Result<Option<String>> {
        if Path::new(input).is_absolute() {
            let exists: Option<String> = self
                .conn
                .query_row("SELECT path FROM repos WHERE path = ?1", [input], |r| r.get(0))
                .optional()?;
            return Ok(exists);
        }
        let q = format!("%{}%", input);
        let row: Option<String> = self
            .conn
            .query_row(
                "SELECT path FROM repos WHERE name LIKE ?1 OR path LIKE ?1 ORDER BY name ASC LIMIT 1",
                [q],
                |r| r.get(0),
            )
            .optional()?;
        Ok(row)
    }

    fn ensure_tag(&self, tag: &str) -> Result<i64> {
        self.conn
            .execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
        let id: i64 = self
            .conn
            .query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))?;
        Ok(id)
    }

    fn repo_id_by_path(&self, repo_path: &str) -> Result<Option<i64>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM repos WHERE path = ?1", [repo_path], |r| r.get(0))
            .optional()?;
        Ok(id)
    }

    fn prune_orphan_tags(&self) -> Result<usize> {
        let n = self.conn.execute(
            "DELETE FROM tags WHERE NOT EXISTS (SELECT 1 FROM repo_tags WHERE tag_id = tags.id)",
            [],
        )?;
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn upsert_and_search() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("t.db");
        let db = Db::open(&db_path)?;
        db.init_schema()?;

        db.upsert_repo(&RepoMeta {
            path: "/tmp/repo-a".to_string(),
            name: "repo-a".to_string(),
            default_branch: Some("main".to_string()),
            last_commit_ts: Some(123),
            last_scan_ts: 456,
            readme_excerpt: Some("hello world".to_string()),
        })?;

        let rows = db.search_repos("hello")?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "repo-a");
        Ok(())
    }

    #[test]
    fn tags_roundtrip() -> Result<()> {
        let dir = tempdir()?;
        let db_path = dir.path().join("t.db");
        let db = Db::open(&db_path)?;
        db.init_schema()?;

        let repo_path = "/tmp/repo-b";
        db.upsert_repo(&RepoMeta {
            path: repo_path.to_string(),
            name: "repo-b".to_string(),
            default_branch: None,
            last_commit_ts: None,
            last_scan_ts: 1,
            readme_excerpt: None,
        })?;

        db.add_tag_to_repo(repo_path, "backend")?;
        db.add_tag_to_repo(repo_path, "backend")?;
        db.add_tag_to_repo(repo_path, "rust")?;

        let tags = db.list_repo_tags(repo_path)?;
        assert_eq!(tags, vec!["backend".to_string(), "rust".to_string()]);

        db.remove_tag_from_repo(repo_path, "backend")?;
        let tags = db.list_repo_tags(repo_path)?;
        assert_eq!(tags, vec!["rust".to_string()]);
        Ok(())
    }
}
