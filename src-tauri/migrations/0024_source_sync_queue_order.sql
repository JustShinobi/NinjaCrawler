-- Ordem manual da fila (drag-and-drop) que sobrevive ao restart. Quanto menor o
-- order_index, mais cedo o job é restaurado/enfileirado. Jobs antigos ficam com
-- 0 e caem para o início; novos jobs recebem MAX(order_index)+1 no enqueue.
ALTER TABLE source_sync_queue_jobs ADD COLUMN order_index INTEGER NOT NULL DEFAULT 0;
