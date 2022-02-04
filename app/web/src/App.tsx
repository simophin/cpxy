import { AppBar, Toolbar, Typography } from '@mui/material';
import { Box } from '@mui/system';
import './App.css';
import UpstreamList from './UpstreamList';

function App() {
  return (
    <Box sx={{ flexGrow: 1 }}>
      <AppBar position='static'>
        <Toolbar>
          <Typography variant='h6'>Proxy admin</Typography>
        </Toolbar>
      </AppBar>

      <UpstreamList />
    </Box >
  );
}

export default App;
