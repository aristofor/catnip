# Tutorial Level ~ 0 minute

## Installation éclair

> Ce guide se lit en *0 minute* si tu lis vite et que tu ne tiens pas compte du temps d'installation. Techniquement
> correct, donc validé en mode speedrun.

```bash
git clone http://framagit.org/aristofor/catnip
cd catnip
make venv && make install
```

Catnip est prêt : extension Rust compilée, moteur chargé.

> Setup terminé. Le cockpit est live.

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

Le fichier est évalué, le résultat drop.

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

## Références

- [CLI](../user/CLI.md)
- [REPL](../user/REPL.md)
