-- Migration 0004 (ADR 0122, the Conversations links bug of 0.1.8): the citations of an agent turn.
-- Forward only (MEM-35a). No row is deleted (U9).
--
-- A turn row (`backend_kind = 'genie_agent'`) keeps the combined answer in final_answer_md, and
-- that text cites each workspace as [n]. `citations` says which workspace each n names: a JSON
-- array of {"n": 1-based number, "profile": CLI profile name}. With it, hub Conversations draws
-- each [n] as a citation chip with its Genie One link. It is local only, like final_answer_md,
-- and it holds no question, answer, token or host. Every other row keeps '[]'.

ALTER TABLE genie_exchange ADD COLUMN citations TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(citations));
