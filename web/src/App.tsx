import { Settings, Terminal } from '@mui/icons-material';
import { AppBar, Button, Toolbar, Typography } from '@mui/material';
import { Box } from '@mui/system';
import { useState } from 'react';
import './App.css';
import UpstreamList from './UpstreamList';
import RawSettingsEdit from './RawSettingsEdit';

function App() {
  const [showSettings, setShowSettings] = useState(false);
  const [showRawSettings, setShowRawSettings] = useState<boolean>(false);

  return (
    <Box sx={{ flexGrow: 1 }}>
      <AppBar position='static'>
        <Toolbar>
          <Typography variant='h6' sx={{ flexGrow: 1 }}>Proxy admin</Typography>

          <Button color="inherit" onClick={() => setShowSettings(true)}><Settings /></Button>
          <Button color="inherit" onClick={() => setShowRawSettings(true)}><Terminal /></Button>
        </Toolbar>
      </AppBar>

      <UpstreamList showSettings={showSettings} onSettingsClosed={() => setShowSettings(false)} />
      {showRawSettings && <RawSettingsEdit onClose={() => setShowRawSettings(false)} />}
    </Box >
  );
}

export default App;
