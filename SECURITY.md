# Política de Segurança

A TYBA executa comandos gerados por agentes de IA no filesystem do usuário — segurança é o núcleo do produto, não um anexo. O modelo de ameaça e as regras estão em [docs/SECURITY.md](docs/SECURITY.md).

## Reportando vulnerabilidades

Por favor, **não abra issue pública** para vulnerabilidades. Use o [GitHub Security Advisories](../../security/advisories/new) (reporte privado) do repositório.

Comprometemo-nos a responder em até 72h e a creditar o reporte no advisory, se desejado.

## Escopo

Especialmente interessados em: bypass de aprovação de ações vermelhas, escape do boundary de worktree, vazamento de env/secrets para sessões de agente, injeção via escape sequences (OSC 52/8, paste), e prompt injection que resulte em ação não aprovada.
