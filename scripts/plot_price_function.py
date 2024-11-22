from collections.abc import Callable
import matplotlib.pyplot as plt
from math import floor, log, log10
import numpy as np

def p(t: int):
    return 1.0001**t

def p_inv(p: float):
    return floor(log(p, 1.0001))

def gen_p(epsilon: float) -> Callable[[int], float] | None:
    if epsilon <= 0: return None
    return lambda t: (1 + epsilon)**t

def gen_p_inv(epsilon: float) -> Callable[[float], int] | None:
    if epsilon <= 0: return None
    return lambda p: floor(log(p, 1 + epsilon))

def osmo_p(t: int):
    d = 9e6
    f = floor(t/d)
    return 10**f + (10**(f-6)) * (t - d*f)

def osmo_p_inv(p: float) -> int | None:
    if p < 0: return None
    z = floor(log10(p))
    return round(10**(6-z)*(p + (9*z - 1)*10**z))

print(osmo_p_inv(1/(10**18)))
print(osmo_p_inv(2**128 - 1))
print(osmo_p_inv((2**128 - 1)/(10**18)))

MIN_TICK = -108_000_000
MAX_TICK = 342_000_000

def explore():
    iter_count = MAX_TICK + abs(MIN_TICK)
    for t in range(MIN_TICK, MIN_TICK + 11):
        expected = t
        p = osmo_p(t)
        got = osmo_p_inv(p)
        # print(f"Done {(t - MIN_TICK)/iter_count}")
        print(expected, got, p)
        if expected != got: assert(false)

            
def test_inv():
    osmo_p_inv_test_cases = {
        0.099998: -9000200,
        0.099999: -9000100,
        0.94998: -500200,
        0.94999: -500100,
        0.99998: -200,
        0.99999: -100,
        1: 0,
        1.0001: 100,
        1.0002: 200,
        9.9999: 8999900,
        10.001: 9000100,
        10.002: 9000200
    }

    for k, v in osmo_p_inv_test_cases.items():
        inv = osmo_p_inv(k)
        assert inv is not None and inv == v
        invinv = round(osmo_p(inv), len(str(k)))
        assert invinv == k

def plot_prices():
    plt.subplot(1, 2, 1)
    ticks = np.linspace(MIN_TICK, MAX_TICK, 1_000_000)
    plt.plot(ticks, list(map(osmo_p, ticks)))

    plt.subplot(1, 2, 2)
    prices = np.logspace(-18, 20, num=50, endpoint=True, base=10.0, dtype=None, axis=0)
    plt.plot(prices, list(map(osmo_p_inv, prices)))
    plt.show()

plot_prices()
