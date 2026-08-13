# No direct path: the robot firewalls all direct robot<->operator traffic, so
# every byte must transit the relay circuit (DCUtR direct upgrade is blocked).
# The deploy->invoke round-trip must still pass, relay-only.
PROFILE_NAME="nat-relay"
PROFILE_DESC="direct robot<->operator path blocked; relay circuit only"
PROFILE_CLASS="gate"
ROBOT_SHAPE='OP_IP=$(getent hosts operator | awk "{print \$1}") && [ -n "$OP_IP" ] && iptables -A OUTPUT -d "$OP_IP" -j DROP && iptables -A INPUT -s "$OP_IP" -j DROP'
OPERATOR_SHAPE=""
