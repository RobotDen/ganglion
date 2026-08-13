# Satellite/cross-continent RTT: fixed 125ms each way = 250ms RTT, zero
# jitter for determinism (jittered latency belongs to the chaos run).
PROFILE_NAME="high-latency"
PROFILE_DESC="250ms RTT (125ms each way), zero jitter"
PROFILE_CLASS="gate"
ROBOT_SHAPE='tc qdisc add dev eth0 root netem delay 125ms'
OPERATOR_SHAPE='tc qdisc add dev eth0 root netem delay 125ms'
