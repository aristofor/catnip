# FILE: catnip/nd.py
"""
ND-recursion support for Catnip.

Non-deterministic recursion enables concurrent computation graphs
without explicit async/await syntax.
"""

# Rust classes
from ._rs import NDFuture, NDRecur, NDScheduler, NDState

# --- NDTopos singleton ---


class NDTopos:
    """
    Empty topos - identity element for ND operations.

    Singleton pattern ensures all @[] literals refer to the same object.
    Falsy to allow easy termination checks in ND-recursion.
    """

    _instance = None

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    @classmethod
    def instance(cls):
        """Get or create the singleton instance."""
        if cls._instance is None:
            cls._instance = cls()
        return cls._instance

    def __repr__(self):
        return "@[]"

    def __str__(self):
        return "@[]"

    def __bool__(self):
        return False

    def __eq__(self, other):
        return isinstance(other, NDTopos)

    def __hash__(self):
        return hash("@[]")

    def __iter__(self):
        return iter(())

    def __len__(self):
        return 0


# --- Worker functions (must stay in Python for pickle serialization) ---


def _worker_init():
    """Initialize a worker process with Catnip registry.

    Called once per worker process via ProcessPoolExecutor initializer.
    Sets up the global registry for lambda reconstruction.
    """
    from .. import Catnip
    from .._rs import set_global_registry

    catnip = Catnip()
    set_global_registry(catnip.registry)


def _worker_execute_simple(seed, nd_lambda):
    """Worker function for process-based execution.

    Creates a local sequential scheduler and recur for the worker process.
    All recursion happens inline within this process.
    """
    from .._rs import NDRecur, NDScheduler

    local_scheduler = NDScheduler(n_workers=1, mode='sequential')
    local_recur = NDRecur(local_scheduler, nd_lambda, None, 'sequential')
    return nd_lambda(seed, local_recur)


__all__ = (
    'NDTopos',
    'NDFuture',
    'NDState',
    'NDRecur',
    'NDScheduler',
    '_worker_init',
    '_worker_execute_simple',
)
