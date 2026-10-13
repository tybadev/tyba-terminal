# Avisos de terceiros

## withfig/autocomplete — base de subcomandos e flags

`src-tauri/src/session/command_spec.tsv` é derivado dos specs do Fig
([withfig/autocomplete](https://github.com/withfig/autocomplete), pacote npm
`@withfig/autocomplete@2.692.3`), gerado por `scripts/gen-command-spec.ts`.

Só entra o estático: nome de subcomando, nome de flag e descrição. Os
`generators` dos specs (que executam comandos) são descartados na extração. As
linhas de `openssl` não vêm do Fig (ver `scripts/gen-openssl-spec.ts`).

Comandos incluídos: `git`, `docker` (com a árvore do `docker-compose` sob
`docker compose`), `pnpm`, `bun`, `yarn`, `npm`, `cargo`, `make`, `brew`,
`ssh`, `hx`, `bat`, `curl`, `gh`, `kubectl`, `ls`, `cat`, `grep`, `find`.

```
MIT License

Copyright (c) 2021 Hercules Labs Inc. (Fig)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
