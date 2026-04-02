# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in FORGE, please report it responsibly.

**Do not open a public issue for security vulnerabilities.**

Instead, please use [GitHub Security Advisories](https://github.com/ncmlabs/forge/security/advisories/new) to report the vulnerability privately.

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

### Response Timeline

- **Acknowledgment:** Within 48 hours
- **Assessment:** Within 1 week
- **Fix:** Depending on severity, typically within 2 weeks for critical issues

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes       |

## Scope

FORGE is a programming language interpreter that makes HTTP calls to LLM providers. Security concerns include but are not limited to:

- Code execution vulnerabilities in the interpreter
- Credential handling (API keys in config files)
- Network request handling
