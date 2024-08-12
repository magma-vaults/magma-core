from collections.abc import Callable
import matplotlib.pyplot as plt
from math import floor, log, log10
import numpy as np

def p1(t: int):
    return 1.0001**t

def p1_inv(p: float):
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
    return 10**f + (t - d*f)*10**(-6+f)

def dec_zeros(x: float):
    return -floor(log10(abs(x))) - 1 if x < .1 else 0

def osmo_p_inv(p: float):
    if p >= 1:
        k = len(str(p).split('.')[0])
        return int((p - 10**k)/(10**(k-7)) + k*9e6)
    else:
        k = dec_zeros(p) + 1
        # return int(1 - (10**-k - p)/(10**(-6-k)) - k*9e6)
        return int(10**(k+6)*p - k*9e6 - 10**6 - 1)

print(osmo_p_inv(10.001))

def plot_prices():
    plt.subplot(1, 2, 1)
    xs1 = np.linspace(0, 800_000, 1_000_000)
    plt.plot(xs1, list(map(p1, xs1)))

    plt.subplot(1, 2, 2)
    xs2 = np.linspace(0, 100_000_000, 1_000_000)
    plt.plot(xs2, list(map(osmo_p, xs2)))
    plt.show()


