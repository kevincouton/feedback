-- Feedback service schema for the lucanian.app fleet.
-- One row per visitor submission. `email` is OPTIONAL and exists solely so
-- the team can reply about that specific submission. It is never required,
-- never shared, and never used for marketing — there are deliberately no
-- marketing columns or flags in this schema.
--
-- The triage columns (classification … theme) are written by the offline
-- w8 triage pass (/root/marketing/scripts/feedback-triage.py), not by this
-- service; they are included here so that pass has a stable target.

CREATE TABLE feedback (
    id TEXT PRIMARY KEY,
    product TEXT NOT NULL,
    page TEXT NOT NULL DEFAULT '',
    category TEXT NOT NULL CHECK (category IN ('useful', 'bug', 'feature')),
    message TEXT NOT NULL,
    email TEXT,
    status TEXT NOT NULL DEFAULT 'received'
        CHECK (status IN ('received', 'in_review', 'planned', 'resolved')),
    honeypot INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- w8 triage write-back columns (offline pass; safe to leave NULL)
    classification TEXT,
    sentiment TEXT,
    confidence REAL,
    triaged_at TEXT,
    github_issue_url TEXT,
    theme TEXT
);

CREATE INDEX idx_feedback_status ON feedback (status);
CREATE INDEX idx_feedback_product ON feedback (product);
CREATE INDEX idx_feedback_created ON feedback (created_at);
