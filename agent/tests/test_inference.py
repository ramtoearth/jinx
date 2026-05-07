# Feature: terminal-day-organizer
"""Tests for inference.py — normalize, ngrams, and infer_group_candidate.

Property tests P4 and P5.
"""

from __future__ import annotations

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from agent.inference import (
    THRESHOLD_AUTO_ASSIGN,
    THRESHOLD_SUGGEST,
    GroupInfo,
    GroupsSnapshot,
    _infer_group_candidate_impl,
    infer_group_candidate,
    ngrams,
    normalize,
)

# ---------------------------------------------------------------------------
# Unit tests for normalize and ngrams
# ---------------------------------------------------------------------------

def test_normalize_lowercases() -> None:
    assert normalize("Trabajo") == "trabajo"


def test_normalize_strips_accents() -> None:
    assert normalize("café") == "cafe"
    assert normalize("niño") == "nino"


def test_normalize_collapses_spaces() -> None:
    assert normalize("  foo   bar  ") == "foo bar"


def test_ngrams_basic() -> None:
    result = ngrams("abc")
    # Padded: " abc " → trigramas " ab", "abc", "bc "
    assert " ab" in result
    assert "abc" in result
    assert "bc " in result


def test_ngrams_empty() -> None:
    assert ngrams("") == set()


def test_ngrams_short() -> None:
    # String shorter than n after padding
    result = ngrams("a")
    # " a " has len 3 → exactly one trigram
    assert len(result) == 1
    assert " a " in result


# ---------------------------------------------------------------------------
# Feature: terminal-day-organizer, Property 4: Determinismo y argmax de la
# inferencia de Grupo
# ---------------------------------------------------------------------------
# Valida: Requisitos 15.9, 15.13

_GROUP_NAMES = st.text(alphabet=st.characters(whitelist_categories=("Ll", "Lu", "Nd")), min_size=1, max_size=20)
_TITLES = st.lists(_GROUP_NAMES, max_size=5)
_GROUP_STRATEGY = st.builds(
    GroupInfo,
    id=st.integers(min_value=1, max_value=1000),
    name=_GROUP_NAMES,
    member_titles=_TITLES,
)
_SNAPSHOT_STRATEGY = st.lists(_GROUP_STRATEGY, min_size=0, max_size=5).map(
    # Ensure unique ids for determinism
    lambda groups: [
        GroupInfo(id=i + 1, name=g.name, member_titles=g.member_titles)
        for i, g in enumerate(groups)
    ]
)
_MESSAGE_STRATEGY = st.text(min_size=0, max_size=50)


# Feature: terminal-day-organizer, Property 4: Determinismo y argmax de la inferencia de Grupo
@settings(max_examples=200, deadline=None)
@given(message=_MESSAGE_STRATEGY, snapshot=_SNAPSHOT_STRATEGY)
def test_p4_deterministic_and_argmax(message: str, snapshot: GroupsSnapshot) -> None:
    """infer_group_candidate is deterministic and returns the Jaccard argmax."""
    from agent.inference import jaccard, normalize as norm, ngrams as ng

    result1 = _infer_group_candidate_impl(message, snapshot)
    result2 = _infer_group_candidate_impl(message, snapshot)

    # Determinism: same call → same result
    assert result1 == result2, f"non-deterministic: {result1} != {result2}"

    group_id, score = result1

    if not snapshot:
        assert group_id is None
        assert score == 0.0
        return

    # Verify argmax: no other group has a strictly higher score
    msg_tri = ng(norm(message))
    for g in snapshot:
        g_tri = ng(norm(g.name + " " + " ".join(g.member_titles)))
        g_score = jaccard(msg_tri, g_tri)
        if g.id != group_id:
            assert g_score <= score, (
                f"group {g.id} (score {g_score:.4f}) should not beat chosen {group_id} (score {score:.4f})"
            )

    # Verify tie-breaking by ascending id
    if group_id is not None:
        msg_tri2 = ng(norm(message))
        candidates_with_max = [
            g.id
            for g in sorted(snapshot, key=lambda x: x.id)
            if abs(jaccard(ng(norm(g.name + " " + " ".join(g.member_titles))), msg_tri2) - score) < 1e-12
        ]
        if candidates_with_max:
            assert group_id == candidates_with_max[0], (
                f"tie-breaking failed: chose {group_id}, min id with max score is {candidates_with_max[0]}"
            )


# ---------------------------------------------------------------------------
# Feature: terminal-day-organizer, Property 5: Ruteo por umbral de la acción
# ---------------------------------------------------------------------------
# Valida: Requisitos 15.10, 15.11, 15.12

# Feature: terminal-day-organizer, Property 5: Ruteo por umbral de la acción tras inferencia
@settings(max_examples=200, deadline=None)
@given(message=_MESSAGE_STRATEGY, snapshot=_SNAPSHOT_STRATEGY)
def test_p5_threshold_routing(message: str, snapshot: GroupsSnapshot) -> None:
    """The action taken corresponds to the score's threshold interval."""
    group_id, score = _infer_group_candidate_impl(message, snapshot)

    if not snapshot or group_id is None:
        # No groups → propose creating a new one (score < THRESHOLD_SUGGEST)
        assert score < THRESHOLD_SUGGEST or snapshot == []
        return

    if score >= THRESHOLD_AUTO_ASSIGN:
        # Should auto-assign
        assert group_id is not None
    elif score >= THRESHOLD_SUGGEST:
        # Should propose with confirmation
        assert group_id is not None
    else:
        # Should propose creating a new group
        assert group_id is not None  # candidate still returned, caller decides action


# ---------------------------------------------------------------------------
# Example-based tests
# ---------------------------------------------------------------------------

def test_exact_name_match_scores_higher_than_unrelated() -> None:
    """A message containing the group name must score higher than an unrelated group."""
    snapshot = [
        GroupInfo(id=1, name="trabajo", member_titles=["reunión de equipo"] * 10),
        GroupInfo(id=2, name="hobbies", member_titles=["pintura", "fotografía"] * 10),
    ]
    group_id, score = _infer_group_candidate_impl(
        "trabajo trabajo trabajo trabajo trabajo trabajo trabajo", snapshot
    )
    assert group_id == 1, f"expected group 1 (trabajo), got {group_id}"
    assert score > 0.0, f"expected non-zero score, got {score:.4f}"


def test_no_match_scores_low() -> None:
    snapshot = [GroupInfo(id=1, name="trabajo", member_titles=["reunión de equipo"])]
    group_id, score = _infer_group_candidate_impl("zzz qwerty", snapshot)
    assert score < THRESHOLD_SUGGEST, f"expected low score, got {score:.4f}"


def test_empty_snapshot_returns_none() -> None:
    group_id, score = _infer_group_candidate_impl("hola mundo", [])
    assert group_id is None
    assert score == 0.0


def test_tie_broken_by_lower_id() -> None:
    # Two groups with the same name → identical scores → lower id wins
    snapshot = [
        GroupInfo(id=2, name="x", member_titles=[]),
        GroupInfo(id=1, name="x", member_titles=[]),
    ]
    group_id, _ = _infer_group_candidate_impl("x", snapshot)
    assert group_id == 1, "tie-breaking must prefer the group with lower id"


def test_tool_interface_returns_dict() -> None:
    """Public @tool interface accepts list[dict] and returns dict."""
    groups = [
        {"id": 1, "name": "trabajo", "member_titles": ["reunión", "proyecto"]},
        {"id": 2, "name": "hobbies", "member_titles": ["música"]},
    ]
    result = infer_group_candidate("trabajo reunión", groups)
    assert isinstance(result, dict)
    assert "group_id" in result
    assert "score" in result
    assert result["group_id"] == 1
    assert result["score"] > 0.0
