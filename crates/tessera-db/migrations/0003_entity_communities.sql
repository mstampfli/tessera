-- Structural community assignment for entities: the connected component of the
-- direct co-occurrence graph an entity belongs to. Entities that are mentioned
-- together form a community; a semantic edge that crosses two communities is a
-- "bridge" (a non-obvious link between things never stated together). Recomputed
-- by the community-detection job; null until first run.
ALTER TABLE entities ADD COLUMN community_id integer;
CREATE INDEX entities_community_idx ON entities (community_id);
