"""Host-side replay of hetero_ensemble.rs's Resonator::step (same update rule,
dt = 1) on unit-impulse trains: periodic vs random gaps of the same mean, for
M173's period-20 design and M174's period-7 design. Tests whether symplectic
Euler's frequency warping explains M174's resonator miss."""
import math, random

def score(period_steps, drive, start, damping=0.01, tau_mult=3.0):
    omega = 2 * math.pi / period_steps
    x = v = e = 0.0
    alpha = min(1.0 / (tau_mult * period_steps), 1.0)
    acc = 0.0
    for s, d in enumerate(drive):
        a = -(omega * omega) * x - 2 * damping * omega * v + d
        v += a
        x += v
        e += alpha * (x * x - e)
        if s >= start:
            acc += math.sqrt(e)
    return acc / (len(drive) - start)

def trains(n, period, lo, hi, start, rng, burst):
    p = [0.0] * n; r = [0.0] * n
    t = 0
    while t < n:
        for b in range(burst):
            if t + b < n: p[t + b] += 1.0
        t += period
    t = rng.randrange(period)
    while t < n:
        for b in range(burst):
            if t + b < n: r[t + b] += 1.0
        t += rng.randint(lo, hi)
    return p, r

def eff_period(period_steps):
    w = 2 * math.pi / period_steps
    return 2 * math.pi / math.acos(1 - w * w / 2)

def tuned_for(true_period):
    # period_steps whose symplectic-Euler resonance lands on true_period
    w_eff = 2 * math.pi / true_period
    w = math.sqrt(2 * (1 - math.cos(w_eff)))
    return 2 * math.pi / w

rng = random.Random(1)
for name, n, period, lo, hi, start, burst in [("M173 design", 800, 20, 10, 30, 400, 2), ("M174 design", 400, 7, 4, 10, 100, 1)]:
    for label, ps in [("period_steps=nominal", float(period)), ("period_steps=warp-corrected", tuned_for(period))]:
        rs = []
        for seed in range(16):
            p, r = trains(n, period, lo, hi, start, rng, burst)
            rs.append((score(ps, p, start), score(ps, r, start)))
        mp = sum(a for a, _ in rs) / 16; mr = sum(b for _, b in rs) / 16
        print(f"{name:12s} {label:28s} ps={ps:6.3f} resonates at {eff_period(ps):6.3f} steps | periodic/random = {mp/mr:5.2f}")
