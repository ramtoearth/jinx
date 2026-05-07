"""Deterministic Group inference — pure functions with no LLM calls.

Implements the trigram-Jaccard algorithm from design.md § "Inferencia de Grupo"
and exposes ``infer_group_candidate`` as a Strands ``@tool``.

Requisitos 15.9–15.13.
"""

from __future__ import annotations

import unicodedata
from typing import Optional

from strands import tool  # type: ignore[import]


# ---------------------------------------------------------------------------
# Text normalisation
# ---------------------------------------------------------------------------

def normalize(s: str) -> str:
    """Lowercase, strip combining marks (NFKD), collapse whitespace."""
    s = unicodedata.normalize("NFKD", s)
    # Discard combining marks (category Mn = Mark, Nonspacing)
    s = "".join(c for c in s if unicodedata.category(c) != "Mn")
    s = s.lower()
    # Collapse runs of whitespace to a single space and strip edges
    s = " ".join(s.split())
    return s


# ---------------------------------------------------------------------------
# Trigram extraction
# ---------------------------------------------------------------------------

def ngrams(s: str, n: int = 3) -> set[str]:
    """Return the set of n-grams of ``s`` with a single space as padding."""
    padded = " " + s + " "
    if len(padded) < n:
        return set()
    return {padded[i : i + n] for i in range(len(padded) - n + 1)}


# ---------------------------------------------------------------------------
# Jaccard score
# ---------------------------------------------------------------------------

def jaccard(a: set[str], b: set[str]) -> float:
    """Jaccard similarity of two sets; returns 0.0 when both are empty."""
    union = a | b
    if not union:
        return 0.0
    return len(a & b) / len(union)


# ---------------------------------------------------------------------------
# GroupsSnapshot type (matches storage::GroupsSnapshot)
# ---------------------------------------------------------------------------

class GroupInfo:
    """Lightweight snapshot of a single Group used for inference."""

    def __init__(self, id: int, name: str, member_titles: list[str]) -> None:
        self.id = id
        self.name = name
        self.member_titles = member_titles


GroupsSnapshot = list[GroupInfo]


# ---------------------------------------------------------------------------
# infer_group_candidate
# ---------------------------------------------------------------------------

def _infer_group_candidate_impl(
    message: str,
    groups_snapshot: GroupsSnapshot,
) -> tuple[Optional[int], float]:
    """Core inference logic over GroupInfo objects."""
    if not groups_snapshot:
        return None, 0.0

    msg_norm = normalize(message)
    msg_tri = ngrams(msg_norm)

    best_id: Optional[int] = None
    best_score = -1.0

    for group in sorted(groups_snapshot, key=lambda g: g.id):
        combined = group.name + " " + " ".join(group.member_titles)
        group_tri = ngrams(normalize(combined))
        score = jaccard(msg_tri, group_tri)
        if score > best_score:
            best_score = score
            best_id = group.id

    return best_id, max(best_score, 0.0)


@tool
def infer_group_candidate(
    message: str,
    groups_snapshot: list[dict],
) -> dict:
    """Infer the best-matching Group for a natural-language message.

    Each entry in ``groups_snapshot`` must have keys ``id`` (int), ``name``
    (str), and ``member_titles`` (list of str).

    Returns ``{"group_id": <int|null>, "score": <float>}``.
    """
    snapshot: GroupsSnapshot = [
        GroupInfo(
            id=int(g["id"]),
            name=str(g["name"]),
            member_titles=[str(t) for t in g.get("member_titles", [])],
        )
        for g in groups_snapshot
    ]
    gid, score = _infer_group_candidate_impl(message, snapshot)
    return {"group_id": gid, "score": score}


# ---------------------------------------------------------------------------
# Threshold constants (Requisitos 15.10, 15.11, 15.12)
# ---------------------------------------------------------------------------

THRESHOLD_AUTO_ASSIGN = 0.75
THRESHOLD_SUGGEST = 0.25
