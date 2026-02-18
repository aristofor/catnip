# FILE: catnip/cli/commands/cache.py
"""Manage Catnip cache."""

from __future__ import annotations

import click

BYTES_PER_MB = 1024 * 1024


def _load_disk_cache_limits():
    """Load disk cache limits from config and return (max_size_bytes, ttl_seconds)."""
    from ...config import ConfigManager

    config = ConfigManager()
    config.load_file()
    max_size_mb = config.get("cache_max_size_mb")
    ttl_seconds = config.get("cache_ttl_seconds")
    max_size_bytes = int(max_size_mb * BYTES_PER_MB) if max_size_mb else None
    return max_size_bytes, ttl_seconds


@click.group("cache")
def cmd_cache():
    """Manage Catnip cache.

    Cache is stored in XDG_CACHE_HOME/catnip (usually ~/.cache/catnip).

    \b
    Examples:
        catnip cache stats
        catnip cache prune
        catnip cache clear
    """
    pass


@cmd_cache.command("stats")
@click.option(
    "--backend",
    type=click.Choice(["disk", "memory"]),
    default="disk",
    help="Cache backend to inspect",
)
def cache_stats(backend):
    """Display cache statistics.

    Shows cache size, hits, misses, hit rate, and volume.
    """
    from ...cachesys import DiskCache, MemoryCache
    from ...config import get_cache_dir

    if backend == "disk":
        cache_dir = get_cache_dir()
        max_size_bytes, ttl_seconds = _load_disk_cache_limits()

        cache = DiskCache(directory=str(cache_dir), max_size_bytes=max_size_bytes, ttl_seconds=ttl_seconds)
        stats = cache.stats()

        click.echo("Disk Cache Statistics")
        click.echo(f"{'=' * 50}")
        click.echo(f"Directory:      {stats['directory']}")
        click.echo(f"Entries:        {stats['size']}")
        click.echo(f"Volume:         {stats['volume_mb']} MB ({stats['volume_bytes']} bytes)")

        if stats.get('max_size_mb'):
            click.echo(f"Max size:       {stats['max_size_mb']} MB")
        else:
            click.echo("Max size:       unlimited")

        if stats.get('ttl_seconds'):
            click.echo(f"TTL:            {stats['ttl_seconds']} seconds")
        else:
            click.echo("TTL:            unlimited")

        click.echo("\nDebug: CATNIP_CACHE_DEBUG=1 catnip ...")
    else:
        # Memory cache stats (primarily for testing)
        cache = MemoryCache()
        stats = cache.stats()

        click.echo("Memory Cache Statistics")
        click.echo(f"{'=' * 50}")
        click.echo(f"Entries:        {stats['size']}")
        if stats.get('max_size'):
            click.echo(f"Max size:       {stats['max_size']}")
        else:
            click.echo("Max size:       unlimited")
        click.echo(f"Hits:           {stats['hits']}")
        click.echo(f"Misses:         {stats['misses']}")
        click.echo(f"Hit rate:       {stats['hit_rate']}")


@cmd_cache.command("prune")
@click.option(
    "--dry-run",
    is_flag=True,
    help="Show what would be removed without actually deleting",
)
def cache_prune(dry_run):
    """Remove expired cache entries.

    Removes entries that exceed TTL or when total cache size exceeds max_size.
    Uses limits from catnip.toml (or unlimited if not configured).
    """
    from ...cachesys import DiskCache
    from ...config import get_cache_dir

    cache_dir = get_cache_dir()
    max_size_bytes, ttl_seconds = _load_disk_cache_limits()

    cache = DiskCache(directory=str(cache_dir), max_size_bytes=max_size_bytes, ttl_seconds=ttl_seconds)

    if dry_run:
        # Get stats before
        stats_before = cache.stats()
        click.echo("Dry run mode - no files will be deleted")
        click.echo(f"Cache directory: {cache_dir}")
        click.echo(f"Current entries: {stats_before['size']}")
        click.echo(f"Current volume:  {stats_before['volume_mb']} MB")

        if ttl_seconds:
            click.echo(f"\nWould remove entries older than {ttl_seconds} seconds")
        if max_size_bytes:
            click.echo(f"Would enforce max size of {max_size_bytes / BYTES_PER_MB:.2f} MB")

        if not ttl_seconds and not max_size_bytes:
            click.echo("\nNo TTL or max_size configured - nothing to prune")
    else:
        stats_before = cache.stats()
        removed = cache.prune()

        stats_after = cache.stats()

        click.echo("Cache pruned successfully")
        click.echo(f"{'=' * 50}")
        click.echo(f"Directory:      {cache_dir}")
        click.echo(f"Removed:        {removed} entries")
        click.echo(f"Before:         {stats_before['size']} entries, {stats_before['volume_mb']} MB")
        click.echo(f"After:          {stats_after['size']} entries, {stats_after['volume_mb']} MB")


@cmd_cache.command("clear")
@click.option(
    "--force",
    is_flag=True,
    help="Skip confirmation prompt",
)
def cache_clear(force):
    """Clear all cache entries.

    WARNING: This permanently deletes all cached data.
    """
    from ...cachesys import DiskCache
    from ...config import get_cache_dir

    cache_dir = get_cache_dir()
    cache = DiskCache(directory=str(cache_dir))

    stats = cache.stats()

    if not force:
        click.echo(f"This will delete all {stats['size']} cache entries ({stats['volume_mb']} MB)")
        click.echo(f"Directory: {cache_dir}")
        if not click.confirm("Are you sure?"):
            click.echo("Aborted")
            return

    cache.clear()
    click.echo("Cache cleared successfully")
    click.echo(f"Removed {stats['size']} entries ({stats['volume_mb']} MB)")
