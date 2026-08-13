# Deterministic loss: iptables statistic nth mode drops EXACTLY every 33rd
# packet (~3%) in each direction — reproducible run-to-run, unlike netem's
# unseedable random loss (which belongs to the chaos run). Fixed 40ms delay,
# zero jitter.
PROFILE_NAME="lossy"
PROFILE_DESC="3% deterministic loss (every 33rd pkt) + fixed 40ms delay"
PROFILE_CLASS="gate"
ROBOT_SHAPE='tc qdisc add dev eth0 root netem delay 40ms && iptables -A OUTPUT -m statistic --mode nth --every 33 --packet 0 -j DROP && iptables -A INPUT -m statistic --mode nth --every 33 --packet 0 -j DROP'
OPERATOR_SHAPE=""
