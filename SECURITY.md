# Security policy

## Supported versions

Until Agentport reaches 1.0, only the latest published version receives security fixes.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting for this repository. Do not open a public issue for vulnerabilities involving command execution, archive extraction, path traversal, unsafe uninstall behavior, credential exposure, or plugin/hook trust.

Include the affected version, platform, source type, reproduction steps, impact, and any suggested mitigation. You should receive an acknowledgement within seven days.

## Trust model

Agentport previews destinations and requires explicit approval before installing detected active content. It does not make third-party packages trustworthy. Users must review package sources and separately trust Codex hooks through `/hooks`.
