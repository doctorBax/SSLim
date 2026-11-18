#!/usr/bin/env python3
"""Static UI integration contracts for dependency-free Blade Android build.

Source contracts complement (do not replace) Android compilation/device tests.
"""
from pathlib import Path

root = Path(__file__).resolve().parents[1] / "android" / "src" / "ru" / "doctorbax" / "blade"

screens = [
    "MainActivity.java",
    "SettingsActivity.java",
    "ProfilesActivity.java",
    "ProfileEditorActivity.java",
    "AppearanceActivity.java",
]

for filename in screens:
    content = (root / filename).read_text(encoding="utf-8")
    assert "BladeMotion.mount(this, scroll);" in content, filename
    assert "BladeMotion.enter(" in content, filename
    assert "BladeMotion.bind(" in content, filename

motion = (root / "BladeMotion.java").read_text(encoding="utf-8")
ambient = (root / "BladeAmbientView.java").read_text(encoding="utf-8")
assert "ValueAnimator.areAnimatorsEnabled()" in motion
assert "onVisibilityAggregated" in ambient
assert all(f"case {i}:" in ambient for i in range(1, 5))
assert "default: orbit(" in ambient
assert "postInvalidateDelayed(55)" in ambient
assert "setClickable(false)" in motion

appearance = (root / "AppearanceActivity.java").read_text(encoding="utf-8")
assert "preserveScroll" in appearance
assert "scroll.scrollTo(0, preserveScroll)" in appearance
assert "BladeHeroView(this)" in appearance
assert "BladeAppearance.setHaptics" in appearance
assert "BladeAppearance.setCustomColor" in appearance

updates = (root / "BladeUpdates.java").read_text(encoding="utf-8")
assert "cancelled.get()" in updates
assert "BladeMotion.show(" in updates

editor = (root / "ProfileEditorActivity.java").read_text(encoding="utf-8")
assert "field.setError(message)" in editor
assert "field.requestFocus()" in editor

print("PASS: shared motion, theme, menu actions, reduced motion, scroll and error contracts")
