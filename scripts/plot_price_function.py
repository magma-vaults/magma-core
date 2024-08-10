from collections.abc import Callable
import matplotlib.pyplot as plt
from math import floor, log
import numpy as np

def p1(t: int):
    return 1.0001**t

def p1_inv(p: float):
    return floor(log(p, 1.0001))

def p2(t: int):
    d = 9e6
    f = floor(t/d)
    return 10**f + (t - d*f)*10**(-6+f)

def gen_p(epsilon: float) -> Callable[[int], float] | None:
    if epsilon <= 0: return None
    return lambda t: (1 + epsilon)**t

def gen_p_inv(epsilon: float) -> Callable[[float], int] | None:
    if epsilon <= 0: return None
    return lambda p: floor(log(p, 1 + epsilon))

print(p2(-108000000))
print(p2(342000000))
print(p2(-342000000))

def plot_prices():
    plt.subplot(1, 2, 1)
    xs1 = np.linspace(0, 800_000, 1_000_000)
    plt.plot(xs1, list(map(p1, xs1)))

    plt.subplot(1, 2, 2)
    xs2 = np.linspace(0, 100_000_000, 1_000_000)
    plt.plot(xs2, list(map(p2, xs2)))
    plt.show()


