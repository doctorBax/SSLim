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

# Sharing a historical profile must preserve its v0.1/v0.2 upgrade
# marker and any private CA certificate; neither is optional for reconnecting.
profile_store = (root / "ProfileStore.java").read_text(encoding="utf-8")
assert '.append("&u=").append(marker)' in profile_store
assert 'link.append("&ca=").append(Base64.getEncoder().encodeToString(' in profile_store
assert 'caPem.getBytes(StandardCharsets.UTF_8)' in profile_store
assert 'else if (key.equals("u")) marker = value' in profile_store
assert 'else if (key.equals("ca"))' in profile_store
# Android must not delete its last good JSON profile database when a rename
# fails, and must recover an interrupted atomic write on startup.
assert 'new AtomicFile(file(context))' in profile_store
assert 'target.startWrite()' in profile_store
assert 'target.finishWrite(output)' in profile_store
assert 'target.failWrite(output)' in profile_store
assert 'store.openRead()' in profile_store
assert 'target.delete()' not in profile_store

vpn = (root / "BladeVpn.java").read_text(encoding="utf-8")
assert 'runTcpSessionAttempt(config.marker)' in vpn
assert 'catch (LegacyUpgradeRejected rejected)' in vpn
assert 'runTcpSessionAttempt("lab-packet/1")' in vpn
assert 'response.indexOf("HTTP/1.1 404 ") == 0' in vpn
assert 'authenticate(input, output, profile, marker)' in vpn

print("PASS: shared motion, theme, menu actions, reduced motion, scroll, errors and profile links")
