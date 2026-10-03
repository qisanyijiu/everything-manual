-- New preparations declare their format; legacy rows remain unknown until read-time inspection.
ALTER TABLE preparations ADD COLUMN format_version INTEGER;
CREATE INDEX preparations_document_updated ON preparations(document_id, updated_at DESC, id DESC);
