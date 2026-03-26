# Tutorial Level ~ 0 minute

Ce guide couvre uniquement l'**installation et le CLI** : compiler, lancer la REPL, exécuter un script. Pour le langage
lui-même, voir le [Guide 2 minutes](QUICKSTART_2MIN.md).

## Prérequis

- **Python** >= 3.12 avec headers (`python3-dev` / `python3-devel`)
- **Rust** stable avec `cargo` ([rustup.rs](https://rustup.rs))
- **uv** ([docs.astral.sh/uv](https://docs.astral.sh/uv))
- **libgmp-dev** (arithmétique précision arbitraire)

```bash
# Debian/Ubuntu
sudo apt install python3-dev libgmp-dev

# Fedora/RHEL
sudo dnf install python3-devel gmp-devel

# macOS (Homebrew)
brew install gmp
```

## Installation éclair

> Ce guide se lit en *0 minute* si tu lis vite et que tu ne tiens pas compte du temps d'installation. Techniquement
> correct, donc validé en mode speedrun.

Pour la release **v0.1.0**, l'installation éclair sera :

```bash
# Catnip standalone
sudo apt install catnip
# Python DSL
pip install catnip-lang
```

En attendant cette release, le chemin le plus court **depuis le dépôt source** est :

```bash
git clone http://framagit.org/aristofor/catnip
cd catnip
make venv && source .venv/bin/activate
make install
```

Catnip est prêt : extension Rust compilée, moteur chargé, package installé dans `.venv`.

> Setup terminé.

______________________________________________________________________

## Premier Contact (30 secondes)

```bash
catnip
```

Puis dans la REPL :

<!-- check: no-check -->

```catnip
▸ 2 + 3
5

▸ f = (x) => { x * 2 }
▸ f(21)
42
```

Si ces trois lignes fonctionnent, l'univers respecte les lois de l'arithmétique **et** Catnip est online.

> Si elles échouent, soit l'installation a raté, soit la physique locale a décidé de muter. Statistiquement :
> l'installation.

______________________________________________________________________

## Exécuter un Script (15 secondes)

Crée un fichier :

```bash
echo '2 + 3' > test.cat
```

Puis exécute-le :

```bash
catnip test.cat
# 5
```

Le fichier est évalué, le résultat tombe.

______________________________________________________________________

## Prochaine Étape

Tu as maintenant :

- une installation fonctionnelle
- une REPL opérationnelle
- un premier script qui tourne

Prochaine étape logique :

**[Guide 2 minutes](QUICKSTART_2MIN.md)** pour apprendre variables, lambdas, boucles, conditions et pattern matching.

> Après ces deux minutes, tu parleras couramment Catnip, un dialecte dimension-agnostique strictement plus stable que le
> français.
