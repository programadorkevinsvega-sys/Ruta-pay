#!/usr/bin/env python3
"""Static acceptance checks for the documentary scope of platform Phase 0."""

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
PHASE0 = DOCS / "phase-0"

required = {
    PHASE0 / "initial-audit.md": [
        "Comprobaciones de línea base",
        "cargo metadata --no-deps --format-version 1",
        "cargo test --workspace",
        "No había pruebas de negocio",
        "no se afirma integración",
    ],
    PHASE0 / "bounded-contexts.md": [
        "identity_access",
        "tenant_registry",
        "payments",
        "gateway_credentials",
    ],
    PHASE0 / "integration-matrix.md": [
        "PostgreSQL",
        "Redis",
        "JetStream",
        "N/A",
        "Fase 1",
        "realtime",
    ],
    PHASE0 / "threat-model.md": [
        "Amenaza",
        "Control",
        "Evidencia",
        "replay",
        "tenant",
        "credenciales",
    ],
}

errors = []
for path, markers in required.items():
    if not path.is_file():
        errors.append(f"missing required document: {path.relative_to(ROOT)}")
        continue
    content = path.read_text(encoding="utf-8")
    for marker in markers:
        if marker.casefold() not in content.casefold():
            errors.append(f"{path.relative_to(ROOT)} is missing required marker: {marker!r}")

for number in range(1, 10):
    matches = list((DOCS / "adr").glob(f"ADR-{number:04d}-*.md"))
    if len(matches) != 1:
        errors.append(
            f"expected exactly one ADR-{number:04d}, found {len(matches)}"
        )

matrix = PHASE0 / "integration-matrix.md"
if matrix.is_file():
    content = matrix.read_text(encoding="utf-8").casefold()
    if "no implementa ni ejecuta runtime" not in content:
        errors.append("integration matrix must explicitly preserve Phase 0's documentary-only scope")

audit = PHASE0 / "initial-audit.md"
if audit.is_file():
    content = audit.read_text(encoding="utf-8").casefold()
    if "0 pruebas" not in content and "sin cobertura" not in content:
        errors.append("initial audit must distinguish successful cargo test from functional coverage")

if errors:
    print("Phase 0 documentary acceptance: FAILED")
    for error in errors:
        print(f"- {error}")
    sys.exit(1)

print("Phase 0 documentary acceptance: PASSED")
print(f"- {len(required)} required audit documents verified")
print("- ADR-0001 through ADR-0009 inventory verified")
print("- documentary-only scope and historical test baseline verified")
