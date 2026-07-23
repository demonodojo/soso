#ifndef _LINUX_NET_TSTAMP_H
#define _LINUX_NET_TSTAMP_H
enum hwtstamp_tx_types { HWTSTAMP_TX_OFF = 0 };
enum hwtstamp_rx_filters { HWTSTAMP_FILTER_NONE = 0 };
struct hwtstamp_config { int flags; int tx_type; int rx_filter; };
#endif
