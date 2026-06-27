-- Guarda o id do post e a data de criação na mídia baixada, para reconstruir o
-- link original (TikTok/IG/X) e agrupar por dia sem depender do nome do arquivo.
-- O ProfileView ainda funciona para mídia antiga (deriva do nome); estas colunas
-- são preenchidas a partir de agora nos novos downloads.
ALTER TABLE provider_sync_media_ledger ADD COLUMN provider_post_key TEXT;
ALTER TABLE provider_sync_media_ledger ADD COLUMN captured_at INTEGER;
