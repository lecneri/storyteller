-- Modo leitura (criança): a sessão fica presa a um leitor até alguém digitar o PIN.
ALTER TABLE acesso.sessoes ADD COLUMN leitor_id uuid REFERENCES acesso.leitores (id) ON DELETE SET NULL;
