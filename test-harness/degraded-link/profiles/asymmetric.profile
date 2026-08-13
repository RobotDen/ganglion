# The under-tested field reality: plentiful downlink, starved uplink (the
# teleop control-ack-starvation case). Robot egress capped hard at 192kbit
# with 30ms delay; operator side untouched. tbf + nested netem are both
# deterministic.
PROFILE_NAME="asymmetric"
PROFILE_DESC="starved uplink: robot egress 192kbit + 30ms; downlink clean"
PROFILE_CLASS="gate"
ROBOT_SHAPE='tc qdisc add dev eth0 root handle 1: tbf rate 192kbit burst 16kb latency 400ms && tc qdisc add dev eth0 parent 1:1 handle 10: netem delay 30ms'
OPERATOR_SHAPE=""
