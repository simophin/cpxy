import { Settings, Terminal } from '@mui/icons-material';
import { AppBar, Button, Toolbar, Typography } from '@mui/material';
import { Box } from '@mui/system';
import { useState } from 'react';
import './App.css';
import LogViewer from './LogViewer';
import UpstreamList from './UpstreamList';

function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [showLogViewer, setShowLogViewer] = useState(false);

  return (
    <Box sx={{ flexGrow: 1 }}>
      <AppBar position='static'>
        <Toolbar>
          <Typography variant='h6' sx={{ flexGrow: 1 }}>Proxy admin</Typography>

          <Button color="inherit" onClick={() => setShowLogViewer(true)}><Terminal /></Button>
          <Button color="inherit" onClick={() => setShowSettings(true)}><Settings /></Button>
        </Toolbar>
      </AppBar>

      <UpstreamList showSettings={showSettings} onSettingsClosed={() => setShowSettings(false)} />

      {showLogViewer && <LogViewer onClose={() => setShowLogViewer(false)} />}

    </Box >
  );
}

export default App;
