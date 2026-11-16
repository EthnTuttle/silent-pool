-- DRAFT Testnet5 integration: additive, no inferred binding of legacy data.
CREATE TABLE pool_chain_identity (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    binding JSONB NOT NULL CHECK (jsonb_typeof(binding) = 'object')
);
