# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.x     | ✅ Yes             |
| < 1.0   | ❌ No              |

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

**Email:** support@au-zone.com  
**Subject:** Security Vulnerability - g2d-rs

### What to Include

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested fixes (optional)

### Response Timeline

- **Acknowledgment:** Within 48 hours
- **Initial Assessment:** Within 5 business days
- **Resolution Timeline:** Communicated after assessment

### What to Expect

1. We will acknowledge receipt of your report
2. We will investigate and assess the vulnerability
3. We will work on a fix if confirmed
4. We will coordinate disclosure timing with you
5. We will credit you in the release notes (unless you prefer anonymity)

### Scope

This security policy applies to:
- The `g2d-sys` crate and any future crates in this repository
- The build and release infrastructure

Issues in the underlying NXP G2D library (`libg2d.so`) should be reported directly to NXP.

## Security Considerations

### FFI Safety

This crate provides **unsafe** FFI bindings. Users must:
- Ensure valid DMA buffer file descriptors
- Properly manage surface lifetimes
- Handle errors from G2D operations

### Dynamic Loading

The library is loaded at runtime via `dlopen`. Users should:
- Only load `libg2d.so` from trusted paths
- Verify library integrity on sensitive systems
