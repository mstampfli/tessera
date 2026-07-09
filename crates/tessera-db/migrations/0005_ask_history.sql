-- A log of ask-with-citations questions and their answers, so a question asked
-- once can be read again later. The full answer (text + resolved citations) is
-- stored as jsonb so the history renders identically to the live answer.
CREATE TABLE ask_history (
    id         uuid PRIMARY KEY,
    principal  text,
    question   text NOT NULL,
    answer     jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ask_history_created_idx ON ask_history (created_at DESC);
