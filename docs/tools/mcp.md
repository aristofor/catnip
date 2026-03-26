# Serveur MCP

Catnip fournit un serveur [MCP](https://modelcontextprotocol.io/) (Model Context Protocol) qui expose le pipeline du
langage -- parsing, inspection, évaluation, debugging -- via un protocole structuré consommable par des agents.

Le serveur est implémenté en Rust pur (`catnip_mcp/`), utilise `PurePipeline` (pas de runtime Python) et communique via
stdio. SDK : [rmcp](https://crates.io/crates/rmcp).

## Intégration agent

Le MCP permet à un agent de :

- **parser du code à plusieurs niveaux**, du tree-sitter brut (niveau 0) jusqu'à l'IR exécutable post-analyse sémantique
  (niveau 2) ;
- **inspecter les représentations internes** : AST, IR, opcodes, variables en scope ;
- **piloter une session de debug** avec breakpoints, stepping (into/over/out) et inspection de l'état ;
- **évaluer des expressions** dans le contexte courant d'une pause.

Tous les échanges passent par des appels MCP structurés (JSON in, JSON out), sans état implicite côté agent. Le serveur
spawne un thread VM par session de debug et communique via canaux `mpsc` -- l'agent n'a pas à gérer de concurrence.

## Installation

```json
{
  "mcpServers": {
    "catnip": {
      "command": "/path/to/catnip/.venv/bin/catnip-mcp"
    }
  }
}
```

Build : `make install-bins` ou `cargo build --release -p catnip_mcp`.

## Tools

### parse_catnip

Parse du code Catnip et retourne la représentation structurée.

| Paramètre | Type          | Défaut | Description        |
| --------- | ------------- | ------ | ------------------ |
| `code`    | string        | requis | Code source Catnip |
| `level`   | integer (0-2) | 1      | Niveau de parsing  |

**Niveaux de parsing** :

- **0** : Parse tree brut (sortie texte tree-sitter)
- **1** : IR après transformation (JSON structuré)
- **2** : IR exécutable après analyse sémantique (JSON structuré)

```json
{"ir": ["…"], "level": 1}
{"parse_tree": "…", "level": 0}
```

### eval_catnip

Evalue du code et retourne le résultat.

| Paramètre | Type   | Défaut | Description                                  |
| --------- | ------ | ------ | -------------------------------------------- |
| `code`    | string | requis | Code source                                  |
| `context` | object | `{}`   | Variables initiales (JSON, supporte nesting) |

```json
{"result_repr": "42", "result_type": "int", "result_json": 42}
```

### check_syntax

Valide la syntaxe sans exécuter.

| Paramètre | Type   | Défaut | Description |
| --------- | ------ | ------ | ----------- |
| `code`    | string | requis | Code source |

```json
{"valid": true, "message": "Syntax is valid"}
{"valid": false, "error": "…"}
```

### format_code

Formate du code avec style configurable.

| Paramètre     | Type    | Défaut | Description           |
| ------------- | ------- | ------ | --------------------- |
| `code`        | string  | requis | Code source           |
| `indent_size` | integer | 4      | Taille d'indentation  |
| `line_length` | integer | 120    | Longueur max de ligne |

```json
{"formatted_code": "…"}
```

## Tools de debug

Le debugger MCP fonctionne en sessions. L'agent ouvre une session, reçoit un `session_id`, puis pilote l'exécution via
ce handle.

### Flux typique

```
debug_start(code, breakpoints=[5, 12])
  → status: "paused", line: 5, locals: {…}

debug_inspect(session_id)
  → locals: {x: "42", items: "[1, 2, 3]"}

debug_step(session_id, mode="over")
  → status: "paused", line: 6, locals: {…}

debug_eval(session_id, expr="x + 1")
  → result: "43"

debug_continue(session_id)
  → status: "paused", line: 12, locals: {…}

debug_continue(session_id)
  → status: "finished", result: "done"
```

### debug_start

Démarre une session de debug. Retourne l'état à la première pause ou à la fin de l'exécution.

| Paramètre     | Type       | Défaut | Description                      |
| ------------- | ---------- | ------ | -------------------------------- |
| `code`        | string     | requis | Code source                      |
| `breakpoints` | array[int] | `[]`   | Lignes de breakpoint (1-indexed) |

### debug_continue

Continue l'exécution jusqu'au prochain breakpoint ou la fin.

| Paramètre    | Type   | Défaut | Description   |
| ------------ | ------ | ------ | ------------- |
| `session_id` | string | requis | ID de session |

### debug_step

Avance d'un pas dans l'exécution.

| Paramètre    | Type                            | Défaut   | Description      |
| ------------ | ------------------------------- | -------- | ---------------- |
| `session_id` | string                          | requis   | ID de session    |
| `mode`       | `"into"` \| `"over"` \| `"out"` | `"into"` | Mode de stepping |

### debug_inspect

Inspecte les variables locales au point de pause courant. N'avance pas l'exécution.

| Paramètre    | Type   | Défaut | Description   |
| ------------ | ------ | ------ | ------------- |
| `session_id` | string | requis | ID de session |

### debug_eval

Evalue une expression dans le scope de la pause courante. L'évaluation se fait dans un contexte isolé : les effets de
bord ne se propagent pas à la session.

| Paramètre    | Type   | Défaut | Description          |
| ------------ | ------ | ------ | -------------------- |
| `session_id` | string | requis | ID de session        |
| `expr`       | string | requis | Expression à évaluer |

### debug_breakpoint

Ajoute ou retire un breakpoint pendant l'exécution.

| Paramètre    | Type                  | Défaut  | Description                 |
| ------------ | --------------------- | ------- | --------------------------- |
| `session_id` | string                | requis  | ID de session               |
| `line`       | integer               | requis  | Numéro de ligne (1-indexed) |
| `action`     | `"add"` \| `"remove"` | `"add"` | Action                      |

### Réponses debug

Toutes les commandes qui avancent l'exécution (`debug_start`, `debug_continue`, `debug_step`) retournent un payload
uniforme :

**Pause** :

```json
{"session_id": "dbg-1", "status": "paused", "line": 5, "col": 0, "locals": {"x": "42"}, "snippet": "x = x + 1"}
```

**Fin** :

```json
{"session_id": "dbg-1", "status": "finished", "result": "done"}
```

**Erreur** :

```json
{"session_id": "dbg-1", "status": "error", "error": "NameError: 'y' is not defined"}
```

**Timeout** (10s sans événement) :

```json
{"session_id": "dbg-1", "status": "timeout"}
```

La session est nettoyée automatiquement à la fin de l'exécution ou en cas d'erreur. Un timeout ne détruit pas la session
: l'agent peut réessayer.

## Ressources

Les ressources exposent de la documentation et des exemples en lecture seule.

| URI                                  | Type     | Description                                                       |
| ------------------------------------ | -------- | ----------------------------------------------------------------- |
| `catnip://examples/{topic}`          | JSON     | Exemples par thème (`basics`, `functions`, `broadcast`, `cfg`, …) |
| `catnip://codex/{category}/{module}` | text     | Exemples d'intégration Python (`web`, `data-analytics`, …)        |
| `catnip://docs/{section}`            | JSON     | Liste des topics disponibles dans une section                     |
| `catnip://docs/{section}/{topic}`    | markdown | Page de documentation (sections : `lang`, `tuto`, `user`)         |

> La documentation est servie directement depuis les fichiers `docs/`. Ce serveur ne génère rien, il transmet. Un proxy
> sans opinion
