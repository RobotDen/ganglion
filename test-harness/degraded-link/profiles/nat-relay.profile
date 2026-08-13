# No direct path: the robot firewalls all direct robot<->operator traffic, so
# every byte must transit the relay circuit (DCUtR direct upgrade is blocked).
# The deploy->invoke round-trip must still pass, relay-only.
#
# The operator's address is the compose file's static assignment
# (ipv4_address: 172.28.0.30) rather than a DNS lookup: the robot container
# starts BEFORE the operator, so `getent hosts operator` races the operator's
# attachment to the network and intermittently resolves nothing.
PROFILE_NAME="nat-relay"
PROFILE_DESC="direct robot<->operator path blocked; relay circuit only"
PROFILE_CLASS="gate"
ROBOT_SHAPE='iptables -A OUTPUT -d 172.28.0.30 -j DROP && iptables -A INPUT -s 172.28.0.30 -j DROP'
OPERATOR_SHAPE=""
