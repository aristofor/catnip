# FILE: catnip/weird_log.py
"""Thin wrapper -- delegates to Rust implementation in catnip._rs."""

import platform
import sys
import traceback as tb_module


def log_weird_error(exc):
    """Log a CatnipWeirdError to disk via Rust. Silent on any failure."""
    try:
        from catnip._rs import log_weird_error_py

        # Collect Python-only info before crossing to Rust
        traceback_frames = _extract_traceback_frames(exc)
        python_tb = _format_python_traceback()

        details = getattr(exc, 'details', None)
        # Convert detail values to strings for Rust
        if details:
            details = {str(k): v for k, v in details.items()}

        log_weird_error_py(
            message=getattr(exc, 'message', str(exc)),
            cause=getattr(exc, 'cause', None),
            details=details,
            filename=getattr(exc, 'filename', None),
            line=_to_int(getattr(exc, 'line', None)),
            column=_to_int(getattr(exc, 'column', None)),
            context=getattr(exc, 'context', None),
            traceback_frames=traceback_frames,
            python_traceback=python_tb,
            python_version=platform.python_version(),
        )
    except Exception:
        pass  # Never mask the original error


def _to_int(val):
    """Convert to int or None (Rust expects Option<u32>)."""
    if val is None:
        return None
    try:
        return int(val)
    except (TypeError, ValueError):
        return None


def _extract_traceback_frames(exc):
    """Extract Catnip traceback frames as tuples for Rust."""
    ct = getattr(exc, 'traceback', None)
    if ct is None:
        return None
    frames = getattr(ct, 'frames', None)
    if frames is None:
        return None
    return [
        (
            getattr(f, 'function', None),
            getattr(f, 'filename', None),
            _to_int(getattr(f, 'line', None)),
        )
        for f in frames
    ]


def _format_python_traceback():
    """Format current Python traceback if available."""
    exc_info = sys.exc_info()
    if exc_info[2] is not None:
        return ''.join(tb_module.format_exception(*exc_info))
    return None
