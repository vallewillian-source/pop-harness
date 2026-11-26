CREATE INDEX ix_fichas_org_status_fechada_em ON fichas (org_id, status, fechada_em);

CREATE INDEX ix_fluxo_execucoes_org_status_no_atual
    ON fluxo_execucoes (org_id, status, no_atual);
