'use client';
import { useState, useEffect, useCallback, useRef } from 'react';
import { fetchAgents, checkHealth, createFirehoseConnection } from '@/lib/api';
import { CONNECTION_STATUS } from '@/lib/constants';

/**
 * Souveraine server connection hook.
 * Manages health checks, agent discovery, WebSocket firehose,
 * and automatic reconnection.
 */
export function useSouveraine() {
  const [agents, setAgents] = useState([]);
  const [selectedAgent, setSelectedAgent] = useState(null);
  const [connectionStatus, setConnectionStatus] = useState(CONNECTION_STATUS.DISCONNECTED);
  const [firehoseEvents, setFirehoseEvents] = useState([]);
  const wsRef = useRef(null);
  const reconnectTimerRef = useRef(null);

  // Health check and agent loading
  const connect = useCallback(async () => {
    setConnectionStatus(CONNECTION_STATUS.CONNECTING);
    try {
      const healthy = await checkHealth();
      if (!healthy) {
        setConnectionStatus(CONNECTION_STATUS.ERROR);
        return;
      }

      const agentList = await fetchAgents();
      setAgents(agentList);
      if (agentList.length > 0 && !selectedAgent) {
        setSelectedAgent(agentList[0]);
      }

      setConnectionStatus(CONNECTION_STATUS.CONNECTED);
    } catch (e) {
      setConnectionStatus(CONNECTION_STATUS.ERROR);
    }
  }, [selectedAgent]);

  // Connect firehose WebSocket
  const connectFirehose = useCallback(() => {
    if (wsRef.current) {
      wsRef.current.close();
    }

    wsRef.current = createFirehoseConnection(
      (event) => {
        setFirehoseEvents(prev => {
          const next = [...prev, { ...event, receivedAt: new Date() }];
          // Keep last 100 events to prevent memory growth
          return next.slice(-100);
        });
      },
      () => {
        console.warn('Firehose disconnected, reconnecting in 5s...');
        reconnectTimerRef.current = setTimeout(connectFirehose, 5000);
      }
    );
  }, []);

  // Initial connection
  useEffect(() => {
    connect();
    return () => {
      if (wsRef.current) wsRef.current.close();
      if (reconnectTimerRef.current) clearTimeout(reconnectTimerRef.current);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Connect firehose after initial connection
  useEffect(() => {
    if (connectionStatus === CONNECTION_STATUS.CONNECTED) {
      connectFirehose();
    }
  }, [connectionStatus, connectFirehose]);

  const refreshAgents = useCallback(async () => {
    try {
      const agentList = await fetchAgents();
      setAgents(agentList);
    } catch (e) {
      console.error('Failed to refresh agents:', e);
    }
  }, []);

  return {
    agents,
    selectedAgent,
    setSelectedAgent,
    connectionStatus,
    firehoseEvents,
    connect,
    refreshAgents,
  };
}
