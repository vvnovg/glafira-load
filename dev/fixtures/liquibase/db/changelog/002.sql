--liquibase formatted sql
--changeset demo:3
CREATE TABLE payments (id bigint NOT NULL, invoice_id bigint NOT NULL, amount decimal(12,2));
